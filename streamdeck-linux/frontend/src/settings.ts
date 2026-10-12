const object = (value: any): value is Record<string, any> =>
  value !== null && typeof value === "object" && !Array.isArray(value);

function same(a: any, b: any): boolean {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b))
    return (
      a.length === b.length && a.every((value, index) => same(value, b[index]))
    );
  if (!object(a) || !object(b)) return false;
  return (
    Object.keys(a).length === Object.keys(b).length &&
    Object.keys(a).every((key) => Object.hasOwn(b, key) && same(a[key], b[key]))
  );
}

/** Apply only edits made relative to the settings the control actually rendered.
 * A newer snapshot may already have updated the draft before this callback runs.
 * Object fields merge recursively; explicitly edited arrays/scalars replace.
 */
export function mergeSettingsEdit(base: any, edited: any, latest: any): any {
  if (same(base, edited)) return structuredClone(latest);
  if (!object(base) || !object(edited)) return structuredClone(edited);
  const result = object(latest) ? structuredClone(latest) : {};
  for (const key of Object.keys(base))
    if (!Object.hasOwn(edited, key)) delete result[key];
  for (const key of Object.keys(edited)) {
    if (Object.hasOwn(base, key) && same(base[key], edited[key])) continue;
    const value = Object.hasOwn(base, key)
      ? mergeSettingsEdit(base[key], edited[key], result[key])
      : structuredClone(edited[key]);
    Object.defineProperty(result, key, {
      value,
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return result;
}
