/**
 * Applies the stored theme before anything is drawn.
 *
 * This runs as a blocking classic script in `<head>`, which is the whole point
 * of it existing as a separate file. React cannot do this job: by the time a
 * component mounts and an effect runs, the browser has already painted the
 * default theme, and every launch would open with a flash of midnight before
 * settling into whatever the user actually chose. Setting the attribute during
 * head parsing means the first paint is already correct.
 *
 * It is a file rather than an inline `<script>` because the packaged app runs
 * under `script-src 'self'` (see `tauri.conf.json`); an inline script is
 * blocked outright there, and the alternative — a CSP hash that has to be
 * regenerated whenever a character in here changes — trades a real security
 * boundary for a formatting hazard.
 *
 * Nothing may be imported here, so the storage key and the theme names are
 * repeated from `lib/preferences.ts` and `lib/theme.ts`. `themeBoot.test.ts`
 * reads this file and those modules and fails if the two ever disagree.
 */

(function applyStoredTheme() {
  try {
    var stored = window.localStorage.getItem('osstat.preferences.v1');
    if (stored === null) return;

    var theme = JSON.parse(stored).theme;
    // Validated rather than trusted: this writes an attribute that CSS selects
    // on, and the value comes from a store a user can hand-edit. An unknown
    // name would match no theme block and leave the interface on the tokens in
    // `@theme` -- which is midnight, so the failure would be invisible until
    // someone wondered why their choice keeps reverting.
    if (['midnight', 'carbon', 'contrast', 'terminal'].indexOf(theme) === -1) return;

    document.documentElement.setAttribute('data-theme', theme);
  } catch {
    // Storage can be unavailable and the contents unparseable. Either way the
    // interface opens on the default theme, which is a correct interface with
    // the wrong colours -- and this script runs before anything exists to
    // report a problem to, so there is nowhere for this to go but ignored.
  }
})();
