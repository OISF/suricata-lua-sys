use std::{env, path::PathBuf, process::Command};

fn main() {
    let host = env::var_os("HOST")
        .map(|host| host.into_string().unwrap())
        .unwrap();
    let target = env::var_os("TARGET")
        .map(|host| host.into_string().unwrap())
        .unwrap();

    let out_dir = env::var_os("OUT_DIR").map(PathBuf::from).unwrap();
    let build_dir = out_dir.join("lua");

    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap();

    // Copy the source into a build directory as we shouldn't build in the
    // source directory.
    let _ = std::fs::remove_dir_all(&build_dir);
    std::fs::create_dir_all(&out_dir).unwrap();
    let opts = fs_extra::dir::CopyOptions {
        copy_inside: true,
        overwrite: true,
        ..Default::default()
    };
    let src_dir = manifest_dir.join("lua");
    fs_extra::dir::copy(&src_dir, &build_dir, &opts).unwrap();

    let mut command = Command::new("make");
    if host.contains("windows") || target.contains("windows") {
        command.arg("mingw");
    }

    // -fPIC is required for building into a shared library; also
    // pass-through CFLAGS and SURICATA_LUA_SYS_CFLAGS to MYCFLAGS.
    command.arg(format!(
        "MYCFLAGS=-fPIC {} {}",
        env::var("CFLAGS").unwrap_or_default(),
        env::var("SURICATA_LUA_SYS_CFLAGS").unwrap_or_default()
    ));

    // We only want the library, not the tool.
    command.arg("a");

    // Don't inherit parent MAKEFLAGS, they may not be suitable for
    // this build.
    command.env_remove("MAKEFLAGS");

    let status = command.current_dir(&build_dir).status().unwrap();
    if !status.success() {
        panic!("build failed");
    }

    println!("cargo:rerun-if-env-changed=SURICATA_LUA_SYS_HEADER_DST");
    println!("cargo:rerun-if-env-changed=SURICATA_LUA_SYS_CFLAGS");
    println!("cargo:rerun-if-env-changed=CFLAGS");

    println!("cargo:rustc-link-lib=static=lua");
    println!("cargo:rustc-link-search=native={}", build_dir.display());

    if let Err(err) = copy_headers(&src_dir) {
        panic!("Failed to copy headers: {:?}", err);
    }
}

fn copy_headers(build_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let dst_dir = if let Some(dst_dir) = env::var_os("SURICATA_LUA_SYS_HEADER_DST") {
        dst_dir
    } else {
        return Ok(());
    };

    let rd = std::fs::read_dir(build_dir)
        .map_err(|err| format!("Failed to open build directory {:?}: {:?}", build_dir, err))?;

    for entry in rd.flatten() {
        let path = entry.path();
        if let Some(ext) = path.extension() {
            if ext == "h" {
                let basename = path
                    .file_name()
                    .ok_or_else(|| format!("Failed to determine basename for path {:?}", path))?;
                let dst = PathBuf::from(&dst_dir).join(basename);

                if should_copy(&path, &dst) {
                    std::fs::copy(&path, &dst).map_err(|err| {
                        format!("Failed to copy {:?} to {:?}: {:?}", &path, &dst, err)
                    })?;
                    println!("cargo:rerun-if-changed={}", path.display());
                }
            }
        }
    }

    Ok(())
}

/// Check if we should copy path to destination.
///
/// - If dst doesn't exist
/// - If path and dst sizes differ
/// - If path is newer than dst
///
/// The idea is to avoid unecessary copying, as that can trigger
/// unnecessary rebuilds of the C code that includes the headers.
fn should_copy(path: &PathBuf, dst: &PathBuf) -> bool {
    let dst_meta = if let Ok(meta) = std::fs::metadata(dst) {
        meta
    } else {
        // Destination path does not exist, copy.
        return true;
    };

    let path_meta = if let Ok(meta) = std::fs::metadata(path) {
        meta
    } else {
        return true;
    };

    // If the sizes are different, copy.
    if path_meta.len() != dst_meta.len() {
        return true;
    }

    // If path is newer than dst, copy. But also copy if we fail to
    // get the time of either path.
    if let Ok(path_modified) = path_meta.modified() {
        if let Ok(dst_modified) = dst_meta.modified() {
            if path_modified > dst_modified {
                return true;
            }
        } else {
            return true;
        }
    } else {
        return true;
    }

    false
}
