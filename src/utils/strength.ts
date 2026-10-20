// A quick estimate shown while typing a password. It only guides the user; nothing depends on it.

export type StrengthScore = 0 | 1 | 2 | 3 | 4;

export interface Strength {
  score: StrengthScore;
  label: string;
}

const LABELS = ["Very weak", "Weak", "Fair", "Strong", "Very strong"] as const;

const COMMON_PIECES = [
  "password", "passw0rd", "contraseña", "123456", "654321", "qwerty", "azerty", "asdfgh", "letmein",
  "welcome", "admin", "iloveyou", "monkey", "dragon", "football", "baseball", "master", "login",
  "abc123", "111111", "000000", "sunshine", "princess", "secret", "changeme",
];

function characterPool(password: string): number {
  let pool = 0;
  if (/[a-z]/.test(password)) pool += 26;
  if (/[A-Z]/.test(password)) pool += 26;
  if (/\d/.test(password)) pool += 10;
  if (/[\x20-\x2f\x3a-\x40\x5b-\x60\x7b-\x7e]/.test(password)) pool += 33;
  if (/[^\x20-\x7e]/.test(password)) pool += 64;
  return Math.max(pool, 2);
}

/** Length that actually adds unpredictability: repeated and consecutive characters count less. */
function effectiveLength(password: string): number {
  let length = 0;
  let previous: number | null = null;
  let previousStep: number | null = null;
  for (const character of password) {
    const code = character.codePointAt(0) ?? 0;
    const step = previous === null ? null : code - previous;
    if (step === 0) {
      length += 0.25;
    } else if (step !== null && Math.abs(step) === 1 && step === previousStep) {
      length += 0.4;
    } else {
      length += 1;
    }
    previous = code;
    previousStep = step;
  }
  return length;
}

export function estimateStrength(password: string): Strength {
  let bits = effectiveLength(password) * Math.log2(characterPool(password));
  const lower = password.toLowerCase();
  if (COMMON_PIECES.some((piece) => lower.includes(piece))) bits *= 0.5;
  if (new Set(password).size <= 2) bits = Math.min(bits, 10);

  const score: StrengthScore = bits < 28 ? 0 : bits < 40 ? 1 : bits < 60 ? 2 : bits < 80 ? 3 : 4;
  return { score, label: LABELS[score] };
}
