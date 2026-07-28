/// <reference types="vite/client" />

// Vite's ambient types, which declare the non-code modules the bundler resolves
// — `./index.css` in `main.tsx` most immediately. TypeScript 7 stopped allowing
// a side-effect import of an asset with no declaration behind it (TS2882),
// so the file the Vite template normally ships was suddenly load-bearing.
