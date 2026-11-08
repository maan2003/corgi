use super::{ensure_tool_shims, read_corgi_toml, Store, ToolRt};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn command_alias_uses_the_effective_executable_and_invalidates_cached_shims() {
    let fixture = Fixture::new();
    let pinned = fixture.store.root.join("pinned-cmake");
    let native = fixture.store.root.join("native-cmake");
    let tool = ToolRt {
        name: "cmake-linux".into(),
        version: "1".into(),
        env: "CMAKE".into(),
        value: pinned.to_string_lossy().into_owned(),
        command: "cmake".into(),
        id: "pin".into(),
        bin: "bin/cmake".into(),
        bins: Vec::new(),
        reads: Vec::new(),
        packages: Vec::new(),
        targets: Vec::new(),
    };
    fs::write(&pinned, "").unwrap();
    fs::write(&native, "").unwrap();

    let shims = ensure_tool_shims(&fixture.store, &[&tool], &[])
        .unwrap()
        .unwrap();
    assert_eq!(fs::read_link(shims.join("cmake")).unwrap(), pinned);
    let environment = vec![("CMAKE".into(), native.to_string_lossy().into_owned())];
    let overridden = ensure_tool_shims(&fixture.store, &[&tool], &environment)
        .unwrap()
        .unwrap();
    assert_ne!(overridden, shims);
    assert_eq!(fs::read_link(overridden.join("cmake")).unwrap(), native);
    assert_eq!(
        ensure_tool_shims(&fixture.store, &[&tool], &environment)
            .unwrap()
            .unwrap(),
        overridden
    );
    assert!(ensure_tool_shims(&fixture.store, &[&tool, &tool], &[]).is_err());
}

#[test]
fn a_bins_tool_puts_each_executable_on_path_and_collisions_fail() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let bin = fixture.store.root.join("coreutils/bin");
    fs::create_dir_all(&bin).unwrap();
    for (name, mode) in [("sed", 0o755), ("expr", 0o555), ("README", 0o644)] {
        fs::write(bin.join(name), "").unwrap();
        fs::set_permissions(bin.join(name), fs::Permissions::from_mode(mode)).unwrap();
    }
    std::os::unix::fs::symlink(bin.join("sed"), bin.join("gsed")).unwrap();
    fs::create_dir(bin.join("subdir")).unwrap();
    assert_eq!(
        super::directory_executables(&bin).unwrap(),
        ["expr", "gsed", "sed"]
    );
    let tool = ToolRt {
        name: "coreutils".into(),
        version: "9.11".into(),
        env: String::new(),
        value: bin.to_string_lossy().into_owned(),
        command: "coreutils".into(),
        id: "coreutils-pin".into(),
        bin: String::new(),
        bins: super::directory_executables(&bin).unwrap(),
        reads: Vec::new(),
        packages: Vec::new(),
        targets: Vec::new(),
    };
    let shims = ensure_tool_shims(&fixture.store, &[&tool], &[])
        .unwrap()
        .unwrap();
    let mut names = fs::read_dir(&shims)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["expr", "gsed", "sed"]);
    assert_eq!(fs::read_link(shims.join("sed")).unwrap(), bin.join("sed"));

    let sed = ToolRt {
        name: "sed".into(),
        env: "SED".into(),
        value: "/elsewhere/sed".into(),
        command: "sed".into(),
        id: "sed-pin".into(),
        bin: "bin/sed".into(),
        bins: Vec::new(),
        reads: Vec::new(),
        version: "4.9".into(),
        packages: Vec::new(),
        targets: Vec::new(),
    };
    assert!(ensure_tool_shims(&fixture.store, &[&tool, &sed], &[]).is_err());
}

#[test]
fn nix_store_tools_need_no_download_and_bins_tools_no_env() {
    let fixture = Fixture::new();
    let accepted = [
        "bins = \"/nix/store/aaaa-coreutils-9.11/bin\"",
        "bin = \"/nix/store/aaaa-cmake-4.1.2/bin/cmake\"\nenv = \"CMAKE\"",
        "path = \"/nix/store/aaaa-protobuf-34.1\"\nenv = \"PROTOC_ROOT\"",
        "bins = \"bin\"\nversion = \"1\"\nurl = \"https://example.com/t.tar.gz\"\nsha256 = \"pin\"",
    ];
    for body in accepted {
        fixture.write_raw(body);
        assert!(read_corgi_toml(&fixture.store.root).is_ok(), "{body}");
    }
    fixture.write_raw(accepted[1]);
    let config = read_corgi_toml(&fixture.store.root).unwrap().unwrap();
    let tool = &config.tools["t"];
    assert_eq!(tool.display_version(), "cmake-4.1.2");
    assert_eq!(
        tool.location(&fixture.store),
        std::path::Path::new("/nix/store/aaaa-cmake-4.1.2/bin/cmake")
    );
    let rejected = [
        // The store path is the identity: no second version or pin.
        "bins = \"/nix/store/aaaa-coreutils-9.11/bin\"\nversion = \"9.11\"",
        "bins = \"/nix/store/aaaa-coreutils-9.11/bin\"\nsha256 = \"pin\"",
        // Absolute paths outside the Nix store are ambient host state.
        "bins = \"/usr/bin\"",
        "bin = \"/nix/store\"\nenv = \"X\"",
        // `bins` exports no single value; `bin` and `path` need one.
        "bins = \"/nix/store/aaaa-coreutils-9.11/bin\"\nenv = \"COREUTILS\"",
        "bin = \"/nix/store/aaaa-cmake-4.1.2/bin/cmake\"",
        // A relative location names a download.
        "bin = \"bin/cmake\"\nenv = \"CMAKE\"",
        "bin = \"bin/cmake\"\nbins = \"bin\"\nenv = \"CMAKE\"\nversion = \"1\"\nurl = \"u\"",
    ];
    for body in rejected {
        fixture.write_raw(body);
        assert!(read_corgi_toml(&fixture.store.root).is_err(), "{body}");
    }
}

#[test]
fn tool_commands_must_be_filenames_for_executable_tools() {
    let fixture = Fixture::new();
    for command in ["", ".", "..", "/cmake", "../cmake", r"..\cmake"] {
        fixture.write_config("bin", Some(command));
        assert!(read_corgi_toml(&fixture.store.root).is_err());
    }
    fixture.write_config("path", Some("cmake"));
    assert!(read_corgi_toml(&fixture.store.root).is_err());
    fixture.write_config("bin", Some("cmake"));
    let config = read_corgi_toml(&fixture.store.root).unwrap().unwrap();
    assert_eq!(
        config.tools["cmake-linux"].command.as_deref(),
        Some("cmake")
    );
    fixture.write_config("bin", None);
    let config = read_corgi_toml(&fixture.store.root).unwrap().unwrap();
    assert_eq!(config.tools["cmake-linux"].command, None);
}

struct Fixture {
    store: Store,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "corgi-tool-shims-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Self {
            store: Store::new(root).unwrap(),
        }
    }

    fn write_config(&self, export: &str, command: Option<&str>) {
        let command = command
            .map(|command| format!("command = {command:?}\n"))
            .unwrap_or_default();
        fs::write(
            self.store.root.join("corgi.toml"),
            format!(
                "[tools.cmake-linux]\nversion = \"1\"\nurl = \"https://example.com/cmake.tar.gz\"\n\
                 sha256 = \"pin\"\n{export} = \"bin/cmake\"\nenv = \"CMAKE\"\n{command}"
            ),
        )
        .unwrap();
    }
}

impl Fixture {
    fn write_raw(&self, body: &str) {
        fs::write(
            self.store.root.join("corgi.toml"),
            format!("[tools.t]\n{body}\n"),
        )
        .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.store.root).unwrap();
    }
}
