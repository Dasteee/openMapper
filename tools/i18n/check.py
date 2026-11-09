#!/usr/bin/env python3
"""Checks that every interface text has a translation in every language.

    python3 tools/i18n/check.py              the check CI runs (exit 1 when something is missing)
    python3 tools/i18n/check.py --todo de    the texts `locales/de.tsv` still lacks, as `English<TAB>` lines

The texts are the English ones of `tr("English", "Русский")` calls and `cmd!(icon, "English", "Русский", ...)`
commands in `src/`. A language's table is `locales/<code>.tsv`: the English, a tab, the translation
(`\\n`, `\\t` and `\\\\` stand for a newline, a tab and a backslash).
"""
import glob, os, re, sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
LANGS = "be ca cs de es fr hi hu id it ja kk ko ms nl pl pt-pt pt sr sv th tl tr uk vi zh-tw zh".split()
STR = r'"((?:[^"\\]|\\.)*)"'
CALL = re.compile(r"\btr\(\s*" + STR + r"\s*,\s*" + STR + r"\s*\)", re.S)
CMD = re.compile(r"\bcmd!\(\s*[\w:]+\s*,\s*" + STR + r"\s*,\s*" + STR, re.S)
PLACEHOLDER = re.compile(r"\{[^{}]*\}|%[a-z]|%\{[^}]*\}")


def unescape_rust(t):
    t = re.sub(r"\\\n\s*", "", t)
    simple = {"n": "\n", "t": "\t", "r": "\r", "0": "\0"}

    def one(m):
        g = m.group(1)
        if g in simple:
            return simple[g]
        if g[0] == "u":
            return chr(int(g[2:-1], 16))
        if g[0] == "x":
            return chr(int(g[1:], 16))
        return g

    return re.sub(r"\\(u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)", one, t, flags=re.S)


def unescape_tsv(t):
    return re.sub(r"\\(.)", lambda m: {"n": "\n", "t": "\t", "\\": "\\"}.get(m.group(1), "\\" + m.group(1)), t)


def escape_tsv(t):
    return t.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t")


def source_texts():
    keys = []
    for f in sorted(glob.glob(os.path.join(ROOT, "src", "**", "*.rs"), recursive=True)):
        src = open(f, encoding="utf8").read()
        for m in list(CALL.finditer(src)) + list(CMD.finditer(src)):
            k = unescape_rust(m.group(1))
            if k not in keys:
                keys.append(k)
    return keys


def table(code):
    path = os.path.join(ROOT, "locales", code + ".tsv")
    out = {}
    if os.path.exists(path):
        for line in open(path, encoding="utf8").read().split("\n"):
            if "\t" in line:
                en, tr = line.split("\t", 1)
                out[unescape_tsv(en)] = unescape_tsv(tr)
    return out


def main():
    keys = source_texts()
    if len(sys.argv) == 3 and sys.argv[1] == "--todo":
        have = table(sys.argv[2])
        for k in keys:
            if k not in have:
                print(escape_tsv(k) + "\t")
        return 0
    bad = 0
    for code in LANGS:
        have = table(code)
        missing = [k for k in keys if k not in have]
        wrong = [k for k in keys if k in have and sorted(PLACEHOLDER.findall(k)) != sorted(PLACEHOLDER.findall(have[k]))]
        stale = [k for k in have if k not in set(keys)]
        for k in missing[:5]:
            print(f"locales/{code}.tsv: no translation of {k!r}")
        for k in wrong[:5]:
            print(f"locales/{code}.tsv: placeholders differ in {k!r}")
        if missing or wrong:
            print(f"locales/{code}.tsv: {len(missing)} missing, {len(wrong)} with other placeholders (of {len(keys)} texts)")
            bad += 1
        if stale:
            print(f"locales/{code}.tsv: {len(stale)} texts no longer in the program (harmless)")
    if bad:
        print("Every interface text is translated into every language: see CONTRIBUTING.md")
        return 1
    print(f"{len(keys)} texts, {len(LANGS)} languages: complete")
    return 0


if __name__ == "__main__":
    sys.exit(main())
