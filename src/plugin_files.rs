include!(concat!(env!("OUT_DIR"), "/plugin_files.rs"));

#[cfg(test)]
mod tests {
    use super::FILES;

    fn file(name: &str) -> &'static str {
        let (_, bytes) = FILES
            .iter()
            .find(|(path, _)| *path == name)
            .unwrap_or_else(|| panic!("{name} is not embedded"));
        std::str::from_utf8(bytes).unwrap()
    }

    fn quoted_after<'a>(text: &'a str, prefix: &str) -> &'a str {
        let rest =
            &text[text.find(prefix).unwrap_or_else(|| panic!("no {prefix}")) + prefix.len()..];
        rest.split('\'').next().unwrap()
    }

    #[test]
    fn embeds_the_plugin_but_not_dotfiles() {
        assert!(FILES.iter().any(|(path, _)| *path == "Info.lua"));
        assert!(FILES.iter().all(|(path, _)| !path.contains(".DS_Store")));
    }

    #[test]
    fn versions_agree_across_cargo_version_and_info_lua() {
        let cargo = env!("CARGO_PKG_VERSION");
        assert_eq!(quoted_after(file("Version.lua"), "PLUGIN = '"), cargo);

        let info = file("Info.lua");
        let number = |key: &str| {
            let rest = &info[info.find(&format!("{key} = ")).unwrap() + key.len() + 3..];
            rest.split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap()
                .to_string()
        };
        let version = format!(
            "{}.{}.{}",
            number("major"),
            number("minor"),
            number("revision")
        );
        assert_eq!(version, cargo);
    }
}
