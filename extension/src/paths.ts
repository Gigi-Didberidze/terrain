import { isAbsolute, relative, sep } from "node:path";

/** Workspace-relative path with '/' separators, or undefined if outside the workspace. */
export function toRelative(root: string, fsPath: string): string | undefined {
  const rel = relative(root, fsPath);
  if (!rel || rel.startsWith("..") || isAbsolute(rel)) return undefined;
  return sep === "/" ? rel : rel.split(sep).join("/");
}
