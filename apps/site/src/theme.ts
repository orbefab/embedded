/** The same theme contract as the main site: localStorage key `theme`, values `dark` and
 * `light`, that class on `<html>`, default dark, no system preference. Both sites share an
 * origin, so a choice on either one is the choice on the other. */
export type ThemeName = "dark" | "light";

export const THEME_KEY = "theme";

/** Runs in `<head>` before the stylesheet, so the first paint already has the stored class. */
export const THEME_INIT = `(function(){var k=${JSON.stringify(THEME_KEY)};function want(){try{var s=localStorage.getItem(k);return s==="light"||s==="dark"?s:"dark";}catch(e){return "dark";}}function apply(){var t=want();var el=document.documentElement;if(el.classList.contains(t)&&!el.classList.contains(t==="dark"?"light":"dark")&&el.style.colorScheme===t)return;el.classList.remove("light","dark");el.classList.add(t);el.style.colorScheme=t;}apply();new MutationObserver(apply).observe(document.documentElement,{attributes:true,attributeFilter:["class"]});window.addEventListener("storage",function(e){if(e.key===k)apply();});})();`;

export function themeName(): ThemeName {
  if (typeof document === "undefined") return "dark";
  return document.documentElement.classList.contains("light") ? "light" : "dark";
}

export function applyTheme(theme: ThemeName) {
  localStorage.setItem(THEME_KEY, theme);
  const root = document.documentElement;
  root.classList.remove("light", "dark");
  root.classList.add(theme);
  root.style.colorScheme = theme;
}
