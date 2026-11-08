use super::*;

const TARGET: &str = "x86_64-linux-android";

/// Target tools in `directory`, run by `interpreter`: `cc` compiles with the
/// supplied host Clang for the target and leaves a mark beside its output,
/// `ar` archives with the host's, and `linker` writes `linked` as the output.
fn write_target_tools(directory: &Path, interpreter: &str, linked: &str) {
    let compiler = std::env::var("CORGI_CC").unwrap();
    let archiver = Path::new(&compiler).with_file_name("ar");
    fs::create_dir_all(directory).unwrap();
    for (name, body) in [
        (
            "cc",
            format!(
                "out=\nfor a in \"$@\"; do [ \"$prev\" = -o ] && out=$a; prev=$a; done\n\
                 : > \"$out.by-target-cc\"\n\
                 exec {compiler:?} --target={TARGET} -nostdlibinc \"$@\"\n"
            ),
        ),
        ("c++", "exit 1\n".to_string()),
        (
            "ar",
            format!("exec {:?} \"$@\"\n", archiver.display().to_string()),
        ),
        (
            "linker",
            format!(
                "while [ $# -gt 0 ]; do [ \"$1\" = -o ] && out=$2; shift; done\n\
                 printf '%s\\n' {linked:?} > \"$out\"\n"
            ),
        ),
    ] {
        let path = directory.join(name);
        fs::write(&path, format!("#!{interpreter}\n{body}")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

/// The test environment's bash, by its Nix store path.
fn store_bash() -> String {
    let path = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|directory| directory.join("bash"))
        .find(|path| path.is_file())
        .expect("the tests need bash on PATH");
    let path = fs::canonicalize(path).unwrap().display().to_string();
    assert!(
        path.starts_with("/nix/store/"),
        "bash is not in the Nix store: {path}"
    );
    path
}

fn with_target_tools(command: &mut Command, directory: &Path) {
    for (variable, name) in [
        ("CORGI_TARGET_CC", "cc"),
        ("CORGI_TARGET_CXX", "c++"),
        ("CORGI_TARGET_AR", "ar"),
        ("CORGI_TARGET_LINKER", "linker"),
    ] {
        command.env(variable, directory.join(name));
    }
}

#[test]
fn supplied_target_tools_compile_build_scripts_c_and_link_target_units_only() {
    let directory = TestDirectory::new("supplied-target");
    let workspace = directory.path.join("workspace");
    let store = directory.path.join("store");
    let tools = directory.path.join("tools");
    write_linux_package(&workspace, &directory.package_name);
    fs::write(
        workspace.join("Cargo.toml"),
        format!(
            "[package]\nname = {:?}\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [lib]\ncrate-type = [\"cdylib\"]\n",
            directory.package_name
        ),
    )
    .unwrap();
    fs::write(
        workspace.join("native.c"),
        "int answer(void) { return 42; }\n",
    )
    .unwrap();
    fs::write(
        workspace.join("corgi.toml"),
        format!(
            "[extra-inputs]\n{} = [\"native.c\"]\n",
            directory.package_name
        ),
    )
    .unwrap();
    // The script runs on the host, linked by the host's compiler; it compiles
    // C for the target with the tools the build hands it.
    fs::write(
        workspace.join("build.rs"),
        r#"use std::{env, path::PathBuf, process::Command};
fn main() {
    let target = env::var("TARGET").unwrap().replace('-', "_");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = out.join("native.o");
    assert!(Command::new(env::var_os(format!("CC_{target}")).unwrap())
        .args(["-c", "native.c", "-o"]).arg(&object).status().unwrap().success());
    assert!(out.join("native.o.by-target-cc").exists(), "CC_{target} is not the target's compiler");
    assert!(Command::new(env::var_os(format!("AR_{target}")).unwrap())
        .arg("crs").arg(out.join("libnative.a")).arg(&object).status().unwrap().success());
    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-lib=static=native");
}
"#,
    )
    .unwrap();
    fs::write(
        workspace.join("src/lib.rs"),
        "unsafe extern \"C\" { fn answer() -> i32; }\n\
         #[unsafe(no_mangle)] pub extern \"C\" fn doubled() -> i32 { unsafe { answer() * 2 } }\n",
    )
    .unwrap();
    let interpreter = store_bash();
    let library = workspace
        .join("target")
        .join(TARGET)
        .join("debug")
        .join(format!(
            "lib{}.so",
            directory.package_name.replace('-', "_")
        ));

    // A changed tool is a changed toolchain: the library is linked again.
    for linked in ["first linker", "second linker"] {
        write_target_tools(&tools, &interpreter, linked);
        let mut command = corgi_store_command(&workspace, "build", ["--target", TARGET], &store);
        with_target_tools(&mut command, &tools);
        let output = command.output().unwrap();
        assert_success(&output, "build for a target with supplied tools");
        assert_eq!(fs::read_to_string(&library).unwrap(), format!("{linked}\n"));
    }

    let output = corgi_store_command(&workspace, "build", ["--target", TARGET], &store)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("CORGI_TARGET_CC"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The host's /usr is not an input of the build's actions.
    write_target_tools(&tools, "/usr/bin/env bash", "unused");
    let mut command = corgi_store_command(&workspace, "build", ["--target", TARGET], &store);
    with_target_tools(&mut command, &tools);
    let output = command.output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("runs with /usr/bin/env, which builds cannot reach"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
