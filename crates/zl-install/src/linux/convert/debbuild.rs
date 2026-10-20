use crate::linux::model::PackageModel;
use crate::linux::util::normalize_arch;

pub fn sanitize_version(version: &str) -> String {
    let mut v: String = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || ".+~:-".contains(c) {
                c
            } else {
                '.'
            }
        })
        .collect();
    while v.ends_with(['.', '-']) {
        v.pop();
    }
    if !v
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
    {
        v = format!("0.{v}");
    }
    if v == "0." {
        v = "0".into();
    }
    v
}

pub fn deb_arch(arch: &str) -> &'static str {
    match normalize_arch(arch).as_str() {
        "any" => "all",
        "aarch64" => "arm64",
        "arm" => "armhf",
        "x86" => "i386",
        _ => "amd64",
    }
}

fn description_field(model: &PackageModel) -> String {
    let summary = model
        .summary
        .clone()
        .unwrap_or_else(|| model.display_name.clone());
    let mut out = format!(
        "Description: {}",
        summary.lines().next().unwrap_or("").trim()
    );
    if let Some(desc) = &model.description {
        for line in desc.lines() {
            let t = line.trim();
            if t.is_empty() {
                out.push_str("\n .");
            } else {
                out.push_str(&format!("\n {t}"));
            }
        }
    }
    out
}

pub fn control(
    model: &PackageModel,
    pkgname: &str,
    depends: &[String],
    installed_size_bytes: u64,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("Package: {pkgname}\n"));
    out.push_str(&format!("Version: {}\n", sanitize_version(&model.version)));
    out.push_str(&format!("Architecture: {}\n", deb_arch(&model.arch)));
    out.push_str("Maintainer: ZLauncher <zlauncher@localhost>\n");
    out.push_str(&format!(
        "Installed-Size: {}\n",
        installed_size_bytes.div_ceil(1024)
    ));
    if !depends.is_empty() {
        out.push_str(&format!("Depends: {}\n", depends.join(", ")));
    }
    out.push_str("Section: misc\nPriority: optional\n");
    if let Some(url) = &model.homepage {
        out.push_str(&format!("Homepage: {url}\n"));
    }
    out.push_str(&format!(
        "X-ZLauncher-Source: {}\n",
        model
            .source_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    out.push_str(&description_field(model));
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(sanitize_version("2.10-9.fc38"), "2.10-9.fc38");
        assert_eq!(sanitize_version("v1.2_3"), "0.v1.2.3");
        assert_eq!(sanitize_version(""), "0");
    }

    #[test]
    fn control_snapshot() {
        let model = crate::linux::convert::pkgbuild::tests::sample_model();
        insta::assert_snapshot!(control(
            &model,
            "discord",
            &["libgtk-3-0t64".into(), "libnotify4".into()],
            300 * 1024 * 1024
        ));
    }
}
