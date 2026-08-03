/**
 * Naming a model the way Rust names it.
 *
 * A session reports `modelName` and nothing else that identifies the file — no
 * path, by design (ADR-012) — so anything asking "is *this* file the one that is
 * loaded?" has to derive the same name from the same rule. Two copies of that
 * rule is two rules: one of them would eventually round a name differently and
 * the answer would be silently wrong rather than loudly broken.
 */

/**
 * A model file's name, the way Rust reports it back.
 *
 * The Rust side names a session from the file stem, so this drops the directory
 * and the extension to match. Both separators are handled because a Windows path
 * is what this application mostly sees and a POSIX one is what its tests and its
 * Linux builds mostly see.
 *
 * @param path A model file's path, absolute or otherwise.
 * @returns The file's stem, which is what a session calls it.
 */
export function stemOf(path: string): string {
  const name = path.slice(Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1);
  const dot = name.lastIndexOf('.');

  // `dot <= 0` rather than `=== -1`: a name that is nothing but an extension
  // keeps it, which is what `Path::file_stem` does with the same input.
  return dot <= 0 ? name : name.slice(0, dot);
}
