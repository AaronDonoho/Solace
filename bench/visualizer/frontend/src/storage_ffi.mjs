// Browser storage can be missing or throw (private windows, blocked site data).
export function load(key) {
  try {
    return localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}

export function save(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch {}
}
