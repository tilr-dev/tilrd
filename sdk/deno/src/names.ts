// Method names, event subjects and subscription patterns (design B5, B6;
// `crates/tilr-bus/src/names.rs`).

const SEGMENT = /^[a-z0-9_-]+$/;

/** Whether `name` is a method name or an event subject: two or more segments. */
export function validSubject(name: string): boolean {
  const segments = name.split(".");
  return segments.length >= 2 && segments.every((segment) => SEGMENT.test(segment));
}

export const validMethod = validSubject;

/** Whether `pattern` is a subscription pattern: segments, `*`, a trailing `>`. */
export function validPattern(pattern: string): boolean {
  const segments = pattern.split(".");
  if (segments.length < 2) return false;
  return segments.every((segment, i) =>
    segment === "*" || (segment === ">" && i === segments.length - 1) || SEGMENT.test(segment)
  );
}

/** Whether the event `subject` matches the subscription `pattern`. */
export function matches(pattern: string, subject: string): boolean {
  const parts = subject.split(".");
  let i = 0;
  for (const part of pattern.split(".")) {
    if (part === ">") return i < parts.length;
    if (i >= parts.length) return false;
    if (part !== "*" && part !== parts[i]) return false;
    i++;
  }
  return i === parts.length;
}
