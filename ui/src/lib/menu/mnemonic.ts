/** Splits `label` at the first case-insensitive occurrence of `letter` (H-19 mnemonics: the
 * trigger button underlines that letter). Falls back to the whole label with no underlined letter
 * if `letter` doesn't occur in it (defensive — every real menu label contains its own mnemonic). */
export interface MnemonicParts {
  before: string;
  letter: string;
  after: string;
}

export function splitMnemonic(label: string, letter: string): MnemonicParts {
  const index = label.toLowerCase().indexOf(letter.toLowerCase());
  if (index === -1) {
    return { before: label, letter: "", after: "" };
  }
  return {
    before: label.slice(0, index),
    letter: label[index]!,
    after: label.slice(index + 1),
  };
}
