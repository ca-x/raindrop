use std::{env, path::Path};

#[path = "build/official_ai.rs"]
mod official_ai;

fn main() {
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
        println!("cargo:rerun-if-changed=web/dist");
        if !Path::new("web/dist/index.html").is_file() {
            panic!(
                "production web bundle is missing web/dist/index.html; run `npm --prefix web run build` before building Raindrop with --release"
            );
        }
    }

    official_ai::build().unwrap_or_else(|error| panic!("official AI bundle build failed: {error}"));
}
