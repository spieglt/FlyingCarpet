use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static LOCALE: Mutex<String> = Mutex::new(String::new());
static TRANSLATIONS: OnceLock<HashMap<String, HashMap<String, String>>> = OnceLock::new();

fn translations() -> &'static HashMap<String, HashMap<String, String>> {
    TRANSLATIONS.get_or_init(|| {
        let mut map = HashMap::new();
        let en: HashMap<String, String> =
            serde_json::from_str(include_str!("locales/en.json")).expect("Invalid en.json");
        map.insert("en".to_string(), en);
        let zh: HashMap<String, String> =
            serde_json::from_str(include_str!("locales/zh.json")).expect("Invalid zh.json");
        map.insert("zh".to_string(), zh);
        let es: HashMap<String, String> =
            serde_json::from_str(include_str!("locales/es.json")).expect("Invalid es.json");
        map.insert("es".to_string(), es);
        map
    })
}

pub fn set_locale(locale: &str) {
    let lang = if locale.starts_with("zh") {
        "zh"
    } else if locale.starts_with("es") {
        "es"
    } else {
        "en"
    };
    let mut loc = LOCALE.lock().expect("Couldn't lock LOCALE mutex");
    *loc = lang.to_string();
}

pub fn t(key: &str, args: &[(&str, &str)]) -> String {
    let locale = LOCALE.lock().expect("Couldn't lock LOCALE mutex");
    let locale = if locale.is_empty() { "en" } else { &locale };
    let translations = translations();

    let template = translations
        .get(locale)
        .and_then(|m| m.get(key))
        .or_else(|| translations.get("en").and_then(|m| m.get(key)))
        .map(|s| s.as_str())
        .unwrap_or(key);

    let mut result = template.to_string();
    for (name, value) in args {
        result = result.replace(&format!("{{{}}}", name), value);
    }
    result
}
