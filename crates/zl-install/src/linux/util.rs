pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = true;
    for c in input.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "package".into()
    } else {
        trimmed
    }
}

pub fn title_case(input: &str) -> String {
    input
        .split(['-', '_', ' '])
        .filter(|s| !s.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn normalize_arch(arch: &str) -> String {
    match arch.trim().to_ascii_lowercase().as_str() {
        "amd64" | "x86_64" | "x86-64" | "x64" => "x86_64".into(),
        "arm64" | "aarch64" => "aarch64".into(),
        "i386" | "i486" | "i586" | "i686" | "x86" => "x86".into(),
        "armhf" | "armv7hl" | "armv7l" | "armv7h" | "arm" => "arm".into(),
        "all" | "noarch" | "any" | "" => "any".into(),
        other => other.into(),
    }
}

pub fn arch_compatible(package_arch: &str, host_arch: &str) -> bool {
    let p = normalize_arch(package_arch);
    p == "any" || p == normalize_arch(host_arch)
}

pub fn data_url(mime: &str, bytes: &[u8]) -> String {
    use base64::Engine;
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

pub fn image_mime(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        Some("image/png")
    } else if lower.ends_with(".svg") || lower.ends_with(".svgz") {
        Some("image/svg+xml")
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("image/jpeg")
    } else if lower.ends_with(".ico") {
        Some("image/x-icon")
    } else {
        None
    }
}

pub fn icon_size_rank(path: &str) -> u32 {
    if path.contains("/scalable/") || path.ends_with(".svg") {
        return 10_000;
    }
    path.split('/')
        .filter_map(|seg| seg.split_once('x'))
        .filter_map(|(w, h)| match (w.parse::<u32>(), h.parse::<u32>()) {
            (Ok(w), Ok(h)) if w == h => Some(w),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Discord PTB (beta)"), "discord-ptb-beta");
        assert_eq!(slugify("  --Obsidian--  "), "obsidian");
        assert_eq!(slugify("!!!"), "package");
    }

    #[test]
    fn arch_normalisation() {
        assert!(arch_compatible("amd64", "x86_64"));
        assert!(arch_compatible("noarch", "aarch64"));
        assert!(!arch_compatible("arm64", "x86_64"));
    }

    #[test]
    fn icon_rank_prefers_bigger_then_scalable() {
        assert!(
            icon_size_rank("usr/share/icons/hicolor/scalable/apps/a.svg")
                > icon_size_rank("usr/share/icons/hicolor/512x512/apps/a.png")
        );
        assert!(
            icon_size_rank("usr/share/icons/hicolor/256x256/apps/a.png")
                > icon_size_rank("usr/share/icons/hicolor/48x48/apps/a.png")
        );
        assert_eq!(icon_size_rank("usr/share/pixmaps/a.png"), 0);
    }
}
