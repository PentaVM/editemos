//! `@font-face` CSS for scenes. System variable fonts only vary their axes when
//! loaded through @font-face, and brand fonts extracted from a site are served
//! from the project's own assets (no network at render time).

use crate::assets;
use crate::store::Project;

fn file_url(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

/// (family, file in C:\Windows\Fonts, weight range, stretch range)
const WINDOWS_VARIABLE: &[(&str, &str, &str, &str)] = &[
    ("Bahnschrift", "bahnschrift.ttf", "300 700", "75% 100%"),
    ("Segoe UI Variable", "SegUIVar.ttf", "300 700", "100% 100%"),
    ("Sitka", "SitkaVF.ttf", "400 700", "100% 100%"),
    ("Cascadia Mono", "CascadiaMono.ttf", "200 700", "100% 100%"),
    ("Cascadia Code", "CascadiaCode.ttf", "200 700", "100% 100%"),
];

pub fn face_css(project: Option<&Project>, families: &[&str]) -> String {
    let mut css = String::new();
    for fam in families {
        // 1) a font file extracted from the brand's site
        if let Some(p) = project {
            if let Some(a) = assets::load(p).into_iter().find(|a| a.kind == "font" && a.tags.iter().any(|t| t.eq_ignore_ascii_case(fam))) {
                css.push_str(&format!("@font-face{{font-family:'{fam}';src:url('{}');font-weight:100 900;}}\n", file_url(&p.assets_dir().join(&a.file))));
                continue;
            }
        }
        // 2) a Windows variable font by file, so weight/width axes animate
        if let Some((_, file, w, s)) = WINDOWS_VARIABLE.iter().find(|(f, ..)| f.eq_ignore_ascii_case(fam)) {
            let path = std::path::Path::new(r"C:\Windows\Fonts").join(file);
            if path.exists() {
                css.push_str(&format!("@font-face{{font-family:'{fam}';src:url('{}');font-weight:{w};font-stretch:{s};}}\n", file_url(&path)));
            }
        }
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_variable_fonts_get_a_face() {
        if std::path::Path::new(r"C:\Windows\Fonts\bahnschrift.ttf").exists() {
            let css = face_css(None, &["Bahnschrift", "Nope Sans"]);
            assert!(css.contains("@font-face") && css.contains("bahnschrift.ttf"), "{css}");
            assert_eq!(css.matches("@font-face").count(), 1);
        }
    }

    #[test]
    fn file_urls() {
        assert_eq!(file_url(std::path::Path::new(r"C:\Windows\Fonts\a.ttf")), "file:///C:/Windows/Fonts/a.ttf");
        assert_eq!(file_url(std::path::Path::new("/usr/share/a.ttf")), "file:///usr/share/a.ttf");
    }
}
