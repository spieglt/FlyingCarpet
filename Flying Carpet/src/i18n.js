let translations = {};
let currentLocale = 'en';

export async function initI18n() {
  // detect locale from localStorage or navigator
  const saved = localStorage.getItem('flyingcarpet_locale');
  if (saved) {
    currentLocale = saved;
  } else {
    const lang = navigator.language || 'en';
    if (lang.startsWith('zh')) {
      currentLocale = 'zh';
    } else if (lang.startsWith('es')) {
      currentLocale = 'es';
    } else {
      currentLocale = 'en';
    }
  }

  // load all translation files
  const locales = ['en', 'zh', 'es'];
  for (const loc of locales) {
    try {
      const resp = await fetch(`/locales/${loc}.json`);
      translations[loc] = await resp.json();
    } catch (e) {
      console.error(`Failed to load ${loc} translations:`, e);
    }
  }

  // sync locale to backend
  try {
    await window.__TAURI__.core.invoke('set_locale', { locale: currentLocale });
  } catch (e) {
    console.error('Failed to set backend locale:', e);
  }
}

export function t(key, args) {
  const dict = translations[currentLocale] || translations['en'] || {};
  const fallback = translations['en'] || {};
  let template = dict[key] || fallback[key] || key;

  if (args) {
    for (const [name, value] of Object.entries(args)) {
      template = template.replaceAll(`{${name}}`, value);
    }
  }
  return template;
}

export function getLocale() {
  return currentLocale;
}

export async function setLocale(locale) {
  currentLocale = locale;
  localStorage.setItem('flyingcarpet_locale', locale);
  applyTranslations();
  // sync to backend
  try {
    await window.__TAURI__.core.invoke('set_locale', { locale });
  } catch (e) {
    console.error('Failed to set backend locale:', e);
  }
}

export function applyTranslations() {
  document.querySelectorAll('[data-i18n]').forEach(el => {
    const key = el.getAttribute('data-i18n');
    const translated = t(key);
    if (el.tagName === 'INPUT' && el.type !== 'radio' && el.type !== 'checkbox') {
      el.placeholder = translated;
    } else {
      el.textContent = translated;
    }
  });
}
