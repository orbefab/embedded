import { useEffect, useState } from "react";
import { applyTheme, themeName, type ThemeName } from "../theme";

/** The dark/light switch, the same control as the main site: a moon or sun, the word Dark or
 * Light, and a short dot sweep. The label waits until mount, because the server cannot see
 * the stored choice. */
export function ThemeToggle({ className = "" }: { className?: string }) {
  const [mounted, setMounted] = useState(false);
  const [theme, setTheme] = useState<ThemeName>("dark");
  const [sweeping, setSweeping] = useState(false);

  useEffect(() => {
    setTheme(themeName());
    setMounted(true);
  }, []);

  const isDark = mounted ? theme !== "light" : true;
  return (
    <button
      aria-label={`Switch to ${isDark ? "light" : "dark"} mode`}
      className={`relative inline-flex min-h-11 items-center gap-2 overflow-hidden border border-input px-3 py-2 font-mono text-[0.6875rem] text-muted-foreground uppercase tracking-[0.16em] transition-colors hover:text-foreground ${className}`}
      onClick={() => {
        const next: ThemeName = isDark ? "light" : "dark";
        setSweeping(true);
        applyTheme(next);
        setTheme(next);
        window.setTimeout(() => setSweeping(false), 360);
      }}
      type="button"
    >
      <span
        aria-hidden="true"
        className="dotfield pointer-events-none absolute inset-0"
        style={{ opacity: sweeping ? 0.9 : 0, transition: "opacity 320ms steps(5, end)" }}
      />
      <span aria-hidden="true" suppressHydrationWarning>
        {isDark ? "☾" : "☀"}
      </span>
      <span suppressHydrationWarning>{isDark ? "Dark" : "Light"}</span>
    </button>
  );
}
