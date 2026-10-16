/**
 * Application shell.
 *
 * Milestone 1 placeholder: a title bar with the theme toggle and a welcome
 * card. The real review cockpit replaces the card; the wiring (typed IPC in
 * `src/ipc/`, Zustand stores mirroring the backend in `src/state/`,
 * token-driven styling) stays.
 */

import { LogoMark } from "./components/LogoMark";
import { useThemeStore } from "./state/themeStore";
import styles from "./App.module.css";

export default function App() {
  const themeMode = useThemeStore((s) => s.mode);
  const toggleTheme = useThemeStore((s) => s.toggle);

  return (
    <div className={styles.app}>
      <header className={styles.titlebar}>
        <div className={styles.brand}>
          <LogoMark size={18} className={styles.logo} aria-hidden />
          <span className={styles.product}>Argus</span>
        </div>
        <button
          type="button"
          className="ghost"
          onClick={toggleTheme}
          aria-label="Toggle theme"
        >
          {themeMode === "dark" ? "Light" : "Dark"} theme
        </button>
      </header>

      <main className={styles.body}>
        <section className={styles.card}>
          <LogoMark size={48} className={styles.cardLogo} aria-hidden />
          <h1 className={styles.title}>Argus</h1>
          <p className={styles.description}>
            A review cockpit for GitLab merge requests and GitHub pull requests.
          </p>
        </section>
      </main>
    </div>
  );
}
