/**
 * localStorage that never throws: private windows, blocked site data and
 * full quotas all read as "nothing stored". Only public data goes here (lot
 * payloads, nonces, transaction hashes); keys and signatures never do.
 */

export function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = window.localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

export function writeJson(key: string, value: unknown): boolean {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
    return true;
  } catch {
    return false;
  }
}

export function removeKey(key: string) {
  try {
    window.localStorage.removeItem(key);
  } catch {
    // storage unavailable: nothing to remove
  }
}
