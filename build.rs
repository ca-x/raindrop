use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use ring::digest::{Context, SHA256};

#[path = "build/official_ai.rs"]
mod official_ai;

const WEB_BUNDLE_DIGEST_ENV: &str = "RAINDROP_WEB_BUNDLE_DIGEST";
const EMBEDDED_WEB_CFG: &str = "raindrop_embedded_web";

fn main() {
    println!("cargo:rustc-check-cfg=cfg({EMBEDDED_WEB_CFG})");
    for path in [
        "web/index.html",
        "web/package.json",
        "web/package-lock.json",
        "web/tsconfig.json",
        "web/vite.config.ts",
        "web/lingui.config.ts",
        "web/public",
        "web/src",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

    if env::var("PROFILE").as_deref() != Ok("debug") {
        println!("cargo:rustc-cfg={EMBEDDED_WEB_CFG}");
        println!("cargo:rerun-if-changed=web/dist");
        if !Path::new("web/dist/index.html").is_file() {
            panic!(
                "production web bundle is missing web/dist/index.html; run `npm --prefix web run build` before building Raindrop with --release"
            );
        }
        emit_web_bundle_digest(Path::new("web/dist"))
            .unwrap_or_else(|error| panic!("embedded Web bundle digest failed: {error}"));
    }

    official_ai::build().unwrap_or_else(|error| panic!("official AI bundle build failed: {error}"));
}

fn emit_web_bundle_digest(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect_files(root, &mut files)?;
    files.sort();
    if files.is_empty() {
        return Err("production Web bundle is empty".to_owned());
    }

    let mut context = Context::new(&SHA256);
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "production Web bundle path is invalid")?
            .to_string_lossy();
        let contents = fs::read(&path).map_err(|_| {
            format!(
                "production Web bundle asset could not be read: {}",
                path.display()
            )
        })?;
        context.update(&(relative.len() as u64).to_le_bytes());
        context.update(relative.as_bytes());
        context.update(&(contents.len() as u64).to_le_bytes());
        context.update(&contents);
    }

    let mut digest = String::with_capacity(64);
    for byte in context.finish().as_ref() {
        write!(&mut digest, "{byte:02x}").map_err(|_| "Web bundle digest could not be encoded")?;
    }
    println!("cargo:rustc-env={WEB_BUNDLE_DIGEST_ENV}={digest}");
    Ok(())
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory).map_err(|_| {
        format!(
            "production Web bundle directory could not be read: {}",
            directory.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|_| "production Web bundle entry could not be read")?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|_| {
            format!(
                "production Web bundle metadata could not be read: {}",
                path.display()
            )
        })?;
        if file_type.is_dir() {
            collect_files(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}
