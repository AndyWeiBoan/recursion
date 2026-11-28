// CSV (and TSV) read into rows, for showing a file as a table: quoted fields may hold the
// separator, quotes ("" for one) and line breaks. The separator is a tab for .tsv, else a comma,
// or a semicolon when the first line has more of those (as spreadsheets in some locales write).

export const isTablePath = (path: string) => /\.(csv|tsv)$/i.test(path);

export function separatorOf(path: string, text: string): string {
  if (/\.tsv$/i.test(path)) return "\t";
  const nl = text.indexOf("\n");
  const first = nl < 0 ? text : text.slice(0, nl);
  const count = (c: string) => first.split(c).length - 1;
  return count(";") > count(",") ? ";" : ",";
}

/** At most `max` rows (the header included); `more`: there were others after them. */
export function parseCsv(text: string, sep: string, max: number): { rows: string[][]; more: boolean } {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;
  let i = 0;
  const t = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
  const end = () => {
    row.push(field);
    field = "";
  };
  while (i < t.length) {
    const c = t[i];
    if (quoted) {
      if (c === '"') {
        if (t[i + 1] === '"') {
          field += '"';
          i += 2;
          continue;
        }
        quoted = false;
      } else field += c;
      i++;
      continue;
    }
    if (c === '"' && field === "") quoted = true;
    else if (c === sep) end();
    else if (c === "\n" || c === "\r") {
      end();
      rows.push(row);
      row = [];
      if (c === "\r" && t[i + 1] === "\n") i++;
      if (rows.length >= max) return { rows, more: i + 1 < t.length && t.slice(i + 1).trim() !== "" };
    } else field += c;
    i++;
  }
  if (field !== "" || row.length) {
    end();
    rows.push(row);
  }
  return { rows, more: false };
}

const NUM = /^\s*[-+]?(\d[\d,_]*)?(\.\d+)?([eE][-+]?\d+)?\s*%?\s*$/;
/** A cell that reads as a number (for lining up on the right, and sorting by value). */
export const isNumber = (s: string) => s.trim() !== "" && NUM.test(s) && /\d/.test(s);
const value = (s: string) => Number(s.replace(/[,_%\s]/g, ""));

/** Rows sorted by column `col` (numbers by value when both are numbers, else as text). */
export function sortRows(rows: string[][], col: number, desc: boolean): string[][] {
  const sorted = [...rows].sort((a, b) => {
    const x = a[col] ?? "";
    const y = b[col] ?? "";
    const r = isNumber(x) && isNumber(y) ? value(x) - value(y) : x.localeCompare(y, undefined, { numeric: true, sensitivity: "base" });
    return desc ? -r : r;
  });
  return sorted;
}
