const MOD_CTRL: &str = "<Ctrl>";
const MOD_ALT: &str = "<Alt>";
const MOD_SHIFT: &str = "<Shift>";
const MOD_SUPER: &str = "<Super>";

pub fn default_hotkey() -> &'static str {
    "<Ctrl><Alt>space"
}

pub fn global_hotkeys_supported() -> bool {
    matches!(std::env::var("XDG_SESSION_TYPE").ok().as_deref(), Some("x11"))
}

pub fn to_gtk_accelerator(input: &str) -> Result<String, &'static str> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("hotkey cannot be empty");
    }

    if trimmed.starts_with('<') {
        return Ok(trimmed.to_string());
    }

    let mut modifiers = Vec::new();
    let mut key: Option<String> = None;

    for token in trimmed.split('+') {
        let normalized = token.trim().to_lowercase();
        if normalized.is_empty() {
            continue;
        }

        match normalized.as_str() {
            "ctrl" | "control" => push_unique(&mut modifiers, MOD_CTRL),
            "alt" => push_unique(&mut modifiers, MOD_ALT),
            "shift" => push_unique(&mut modifiers, MOD_SHIFT),
            "super" | "meta" | "win" | "logo" => push_unique(&mut modifiers, MOD_SUPER),
            _ => {
                if key.is_some() {
                    return Err("hotkey must have exactly one key");
                }
                key = Some(normalized);
            }
        }
    }

    let Some(key) = key else {
        return Err("hotkey must include a non-modifier key");
    };

    let mut output = String::new();
    for modifier in modifiers {
        output.push_str(modifier);
    }
    output.push_str(&key);

    Ok(output)
}

fn push_unique(target: &mut Vec<&'static str>, value: &'static str) {
    if !target.contains(&value) {
        target.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::to_gtk_accelerator;

    #[test]
    fn parses_human_readable_hotkey() {
        let parsed = to_gtk_accelerator("Ctrl+Alt+R").expect("hotkey should parse");
        assert_eq!(parsed, "<Ctrl><Alt>r");
    }

    #[test]
    fn preserves_existing_gtk_format() {
        let parsed = to_gtk_accelerator("<Ctrl><Shift>space").expect("gtk accel should parse");
        assert_eq!(parsed, "<Ctrl><Shift>space");
    }

    #[test]
    fn rejects_hotkey_with_only_modifiers() {
        let err = to_gtk_accelerator("Ctrl+Alt").expect_err("must fail without key");
        assert_eq!(err, "hotkey must include a non-modifier key");
    }
}
