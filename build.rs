// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=SugargliderUI/");

    // Link to private macOS frameworks
    println!(
        "cargo:rustc-link-search=framework={}",
        "/System/Library/PrivateFrameworks"
    );

    // Build and link SugargliderUI Swift library
    build_swift_ui();
}

fn build_swift_ui() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let swift_package_dir = PathBuf::from(&manifest_dir).join("SugargliderUI");

    // Only build if the Swift package exists
    if !swift_package_dir.exists() {
        return;
    }

    let profile = env::var("PROFILE").unwrap();
    let swift_config = if profile == "release" {
        "release"
    } else {
        "debug"
    };

    // Build Swift package
    let status = Command::new("swift")
        .args(["build", "-c", swift_config])
        .current_dir(&swift_package_dir)
        .status()
        .expect("Failed to build Swift package");

    if !status.success() {
        panic!("Swift build failed");
    }

    // Link the Swift library
    let swift_build_dir = swift_package_dir.join(".build").join(swift_config);

    println!("cargo:rustc-link-search=native={}", swift_build_dir.display());
    println!("cargo:rustc-link-lib=dylib=SugargliderUI");

    // Set rpath so the dylib can be found at runtime
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", swift_build_dir.display());
}
