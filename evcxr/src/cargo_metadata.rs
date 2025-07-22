// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::eval_context::Config;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use json::JsonValue;
use json::{self};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

/// Returns the library names for the direct dependencies of the crate rooted at
/// the specified path.
pub(crate) fn get_library_names(config: &Config) -> Result<Vec<String>> {
    let output = config
        .cargo_command("metadata")
        .arg("--format-version")
        .arg("1")
        .output()
        .with_context(|| "Error running cargo metadata")?;
    if output.status.success() {
        library_names_from_metadata(std::str::from_utf8(&output.stdout)?)
    } else {
        bail!(
            "cargo metadata failed with output:\n{}{}",
            std::str::from_utf8(&output.stdout)?,
            std::str::from_utf8(&output.stderr)?,
        )
    }
}

pub(crate) fn validate_dep(dep: &str, dep_config: &str, config: &Config) -> Result<()> {
    std::fs::write(
        config.crate_dir().join("Cargo.toml"),
        format!(
            r#"
    [package]
    name = "evcxr_dummy_validate_dep"
    version = "0.0.1"
    edition = "2024"

    [lib]
    path = "lib.rs"

    [dependencies]
    {dep} = {dep_config}
    "#
        ),
    )?;
    let mut cmd = config.cargo_command("metadata");
    let output = cmd.arg("--format-version=1").output()?;
    if output.status.success() {
        static NO_LIB_PATTERN: Lazy<Regex> = Lazy::new(|| {
            Regex::new("ignoring invalid dependency `(.*)` which is missing a lib target").unwrap()
        });
        let stderr = String::from_utf8_lossy(&output.stderr);
        if let Some(captures) = NO_LIB_PATTERN.captures(&stderr) {
            bail!("Dependency `{}` is missing a lib target", &captures[1]);
        }
        Ok(())
    } else {
        static IGNORED_LINES_PATTERN: Lazy<Regex> =
            Lazy::new(|| Regex::new("required by package `evcxr_dummy_validate_dep.*").unwrap());
        static PRIMARY_ERROR_PATTERN: Lazy<Regex> =
            Lazy::new(|| Regex::new("(.*) as a dependency of package `[^`]*`").unwrap());
        let mut message = Vec::new();
        let mut suggest_offline_mode = false;
        for line in String::from_utf8_lossy(&output.stderr).lines() {
            if let Some(captures) = PRIMARY_ERROR_PATTERN.captures(line) {
                message.push(captures[1].to_string());
            } else if !IGNORED_LINES_PATTERN.is_match(line) {
                message.push(line.to_owned());
            }

            if line.contains("failed to fetch `https://github.com/rust-lang/crates.io-index`") {
                suggest_offline_mode = true;
            }
        }

        if suggest_offline_mode {
            message.push("\nTip: Enable offline mode with `:offline 1`".to_owned());
        }
        bail!(message.join("\n"));
    }
}

fn library_names_from_metadata(metadata: &str) -> Result<Vec<String>> {
    let metadata = json::parse(metadata)?;
    let mut direct_dependencies = Vec::new();
    let mut crate_to_library_names = HashMap::new();
    if let (JsonValue::Array(packages), Some(main_crate_id)) = (
        &metadata["packages"],
        metadata["workspace_members"][0].as_str(),
    ) {
        for package in packages {
            if let (Some(package_name), Some(id)) =
                (package["name"].as_str(), package["id"].as_str())
            {
                if id == main_crate_id
                    && let JsonValue::Array(dependencies) = &package["dependencies"]
                {
                    for dependency in dependencies {
                        if let Some(dependency_name) = dependency["name"].as_str() {
                            direct_dependencies.push(dependency_name);
                        }
                    }
                }
                if let JsonValue::Array(targets) = &package["targets"] {
                    for target in targets {
                        if let JsonValue::Array(kinds) = &target["kind"]
                            && kinds.iter().any(|kind| kind == "lib")
                            && let Some(target_name) = target["name"].as_str()
                        {
                            crate_to_library_names.insert(package_name, target_name.to_owned());
                        }
                    }
                }
            }
        }
    }
    let mut library_names = Vec::new();
    for dep_name in direct_dependencies {
        if let Some(lib_name) = crate_to_library_names.get(dep_name) {
            library_names.push(lib_name.replace('-', "_"));
        }
    }
    Ok(library_names)
}

/// Match the pattern or return an error.
macro_rules! prism {
    ($pattern:path, $rhs:expr, $msg:literal) => {
        if let $pattern(x) = $rhs {
            x
        } else {
            bail!("Parse error in Cargo.toml: {}", $msg)
        }
    };
}

/// Parse the crate at given path, producing crate name
pub fn parse_crate_name(path: &str) -> Result<String> {
    use toml::Value;

    let config_path = std::path::Path::new(path).join("Cargo.toml");
    let content = std::fs::read_to_string(config_path)?;
    let package = content
        .parse::<toml::Table>()
        .context("Can't parse Cargo.toml")?;

    // https://doc.rust-lang.org/cargo/reference/manifest.html
    // The fields to define a package are 'package' and 'workspace'
    if let Some(package) = package.get("package") {
        let package = prism!(Value::Table, package, "expected 'package' to be a table");
        let name = prism!(Some, package.get("name"), "no 'name' in package");
        let name = prism!(Value::String, name, "expected 'name' to be a string");
        Ok(name.clone())
    } else if let Some(_workspace) = package.get("workspace") {
        bail!("Workspaces are not supported");
    } else {
        bail!("Unexpected Cargo.toml format: not package or workspace")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval_context::Config;
    use anyhow::Result;
    use std::path::Path;
    use std::path::PathBuf;

    #[test]
    fn test_library_names_from_metadata() {
        assert_eq!(
            library_names_from_metadata(include_str!("testdata/sample_metadata.json")).unwrap(),
            vec!["crate1"]
        );
    }

    fn create_crate(path: &Path, name: &str, deps: &str) -> Result<()> {
        let src_dir = path.join("src");
        std::fs::create_dir_all(&src_dir)?;
        std::fs::write(
            path.join("Cargo.toml"),
            format!(
                r#"
            [package]
            name = "{name}"
            version = "0.0.1"
            edition = "2024"

            [dependencies]
            {deps}
        "#
            ),
        )?;
        std::fs::write(src_dir.join("lib.rs"), "")?;
        Ok(())
    }

    fn path_to_string(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "\\\\")
    }

    #[test]
    fn valid_dependency() -> Result<()> {
        let tempdir = tempfile::tempdir()?;
        let crate1 = tempdir.path().join("crate1");
        let crate2 = tempdir.path().join("crate2");
        create_crate(&crate1, "crate1", "")?;
        create_crate(
            &crate2,
            "crate2",
            &format!(r#"crate1 = {{ path = "{}" }}"#, path_to_string(&crate1)),
        )?;
        let mut config = Config::new(crate2, PathBuf::from("/dummy_evcxr_bin"))?;
        // We allow static linking so that we don't need a valid RUSTC_WRAPPER, since we just passed
        // /dummy_evcxr_bin.
        config.allow_static_linking = true;
        assert_eq!(get_library_names(&config)?, vec!["crate1".to_owned()]);
        Ok(())
    }

    #[test]
    fn invalid_feature() -> Result<()> {
        let tempdir = tempfile::tempdir()?;
        let crate1 = tempdir.path().join("crate1");
        let crate2 = tempdir.path().join("crate2");
        create_crate(&crate1, "crate1", "")?;
        create_crate(
            &crate2,
            "crate2",
            &format!(
                r#"crate1 = {{ path = "{}", features = ["no_such_feature"] }}"#,
                path_to_string(&crate1)
            ),
        )?;
        // Make sure that the problematic feature "no_such_feature" is mentioned
        // somewhere in the error message.
        let mut config = Config::new(crate2, PathBuf::from("/dummy_evcxr_bin"))?;
        config.allow_static_linking = true;
        assert!(
            get_library_names(&config)
                .unwrap_err()
                .to_string()
                .contains("no_such_feature")
        );
        Ok(())
    }
}
