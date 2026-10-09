#!/usr/bin/env python3
"""Generate large dummy `.jw` projects for performance testing.

Standard library only; deterministic for a given --seed.

    python3 tools/gen_bench_jw.py                    # 50k, 100k, 500k, 1M words -> bench_data/
    python3 tools/gen_bench_jw.py --sizes 20000 --out /tmp/x

Each project holds the target number of manuscript words split over chapters (grouped into parts),
and exercises everything the editor has to cope with:

* `$$` notes in all three shapes, stored the way JustWrite stores them (U+E000 line note,
  U+E001..U+E002 inline note, U+E003 / U+E004 block note on their own lines), plus a few literal
  `$$` that are plain text;
* `*italic*`, `**bold**`, dialogue, scene-break lines;
* review comments (Word-style, anchored to a quoted span) in comments.json, with tags, a mix of
  open/resolved, and replies;
* a few note pages (Notes & research), which do not count towards the manuscript words.

Word counts in manifest.json use the same rule as `notes::word_count` in src/core/notes.rs, and the
script re-reads every archive it wrote and checks counts and comment anchors before reporting.
"""
import argparse
import json
import random
import sys
import time
import zipfile
from pathlib import Path

LINE, OPEN, CLOSE, BLOCK_OPEN, BLOCK_CLOSE = "", "", "", "", ""
TAGS = ["plot", "continuity", "prose", "research"]
CONTEXT = 40

WORDS = """
rain night house river window door morning light shadow garden road mill field wind stone
candle table letter voice silence storm harbor bridge street kitchen stair hallway attic cellar
lantern train station city village forest hill valley lake shore boat sail engine clock mirror
coat glove hat boots basket bread wine tea coffee supper dinner breakfast apple orchard barn
horse dog cat bird crow sparrow owl fox wolf deer child mother father sister brother uncle aunt
grandmother neighbour stranger doctor teacher captain soldier sailor farmer baker clerk widow
walked ran waited listened watched whispered shouted laughed wept turned opened closed carried
dropped found lost remembered forgot believed doubted feared hoped wanted needed asked answered
followed returned arrived departed gathered scattered climbed descended crossed lingered hurried
slowly quietly suddenly carefully gently bitterly warmly coldly softly sharply barely nearly
old young tall small heavy pale dark bright narrow wide broken quiet restless tired patient
empty crowded distant familiar strange gentle bitter golden silver grey crooked hollow weathered
and but while although because before after until though where when as if so then still yet
the the the a a an of in on at by with from over under through behind beyond against between
she he they we you it her his their our its this that these those some every each no
was were had has would could might should must did does will can may shall
""".split()

OPENERS = ["The", "A", "Nobody", "Somewhere", "After", "Before", "By morning", "Once", "Still", "Outside"]
NOTE_TEXT = [
    "check the timeline here", "revisit this scene", "is the weather consistent with chapter two",
    "tighten this paragraph", "who knows about the letter at this point", "research: period clothing",
    "foreshadowing - keep", "cut if pacing drags", "name of the ferry? decide", "continuity: left hand or right",
]
COMMENT_BODY = [
    "This reads slowly. Consider cutting a sentence.", "Does she know this yet?", "Lovely image, keep it.",
    "Check the dates against the earlier chapter.", "Too many adverbs in this stretch.",
    "Can we show this instead of telling it?", "Repetition of 'quiet' from the previous page.",
    "Research needed: would a farmer have owned this in that decade?", "Strong ending to the scene.",
    "Voice slips here.",
]
REPLY_BODY = ["Fixed.", "Agreed, rewriting.", "Leaving as is on purpose.", "Moved to chapter four.", "Good catch."]


def word_count(neutral: str) -> int:
    """Port of `notes::word_count` (src/core/notes.rs): words outside notes, containing a letter or digit."""
    count = 0
    in_word = alnum = inline = line_note = block = False
    for c in neutral:
        if c == BLOCK_OPEN:
            block = True
        elif c == BLOCK_CLOSE:
            block = False
        elif block:
            continue
        elif c == "\n":
            line_note = inline = False
            if in_word and alnum:
                count += 1
            in_word = alnum = False
        elif line_note:
            continue
        elif c in (LINE, OPEN):
            if c == LINE:
                line_note = True
            else:
                inline = True
            if in_word and alnum:
                count += 1
            in_word = alnum = False
        elif c == CLOSE:
            inline = False
        elif inline:
            continue
        elif c.isspace():
            if in_word and alnum:
                count += 1
            in_word = alnum = False
        else:
            in_word = True
            alnum = alnum or c.isalnum()
    if in_word and alnum:
        count += 1
    return count


class Gen:
    def __init__(self, seed: int):
        self.r = random.Random(seed)
        self._ids = set()

    def new_id(self) -> str:
        while True:
            i = "%012x" % self.r.getrandbits(48)
            if i not in self._ids:
                self._ids.add(i)
                return i

    def sentence(self, n: int) -> str:
        """A sentence of exactly n whitespace-separated words (n >= 1)."""
        r = self.r
        ws = [r.choice(WORDS) for _ in range(n)]
        ws[0] = ws[0].capitalize()
        # Light markup on a single word: italic / bold.
        roll = r.random()
        if n > 2 and roll < 0.06:
            i = r.randrange(1, n)
            ws[i] = f"*{ws[i]}*"
        elif n > 2 and roll < 0.09:
            i = r.randrange(1, n)
            ws[i] = f"**{ws[i]}**"
        if n > 3 and r.random() < 0.25:
            i = r.randrange(1, n - 1)
            ws[i] += ","
        text = " ".join(ws)
        if r.random() < 0.18 and n >= 3:
            text = "“" + text + ("?”" if r.random() < 0.4 else ".”")
        else:
            text += r.choice([".", ".", ".", ".", "!", "?"])
        return text

    def paragraph(self, words: int) -> str:
        """A paragraph of exactly `words` prose words."""
        out, left = [], words
        while left > 0:
            n = min(left, self.r.randint(5, 20))
            out.append(self.sentence(n))
            left -= n
        return " ".join(out)

    def chapter(self, target_words: int):
        """Returns (text, comment_candidates) for a chapter with exactly target_words words.

        comment_candidates: (start, end) char spans of 3-9 plain prose words, in the stored text.
        """
        r = self.r
        parts, candidates, offset, left = [], [], 0, target_words

        def add(piece: str):
            nonlocal offset
            parts.append(piece)
            offset += len(piece)

        while left > 0:
            roll = r.random()
            if roll < 0.04 and parts:
                add(r.choice(["***", "* * *", "#"]) + "\n\n")
                continue
            if roll < 0.12:  # line note on its own line
                add(LINE + " " + r.choice(NOTE_TEXT) + "\n\n")
                continue
            if roll < 0.15:  # block note
                add(BLOCK_OPEN + "\n" + r.choice(NOTE_TEXT) + "\n" + r.choice(NOTE_TEXT) + "\n" + BLOCK_CLOSE + "\n\n")
                continue
            n = min(left, r.randint(30, 140))
            para = self.paragraph(n)
            left -= n
            kind = r.random()
            if kind < 0.10 and n >= 12:
                # Inline note between two words; the note text is not counted.
                ws = para.split(" ")
                i = r.randrange(1, len(ws) - 1)
                ws[i] = ws[i] + " " + OPEN + r.choice(NOTE_TEXT) + CLOSE
                para = " ".join(ws)
            elif kind < 0.13:
                # A literal marker the author wants as text is stored as plain characters.
                para += " It cost $$ in the end."
                left -= 5  # It cost $$ in the end. -> 5 counted words ("$$" has no letter or digit)
                left = max(left, 0)
            if n >= 12 and OPEN not in para and "*" not in para and "$$" not in para and r.random() < 0.6:
                ws = para.split(" ")
                i = r.randrange(0, len(ws) - 8)
                span = ws[i:i + r.randint(3, 9)]
                start = offset + len(" ".join(ws[:i])) + (1 if i else 0)
                candidates.append((start, start + len(" ".join(span))))
            add(para + "\n\n")
        text = "".join(parts)
        return text, candidates


def trim_to(text: str, candidates, target: int, gen: Gen):
    """The literal '$$' paragraph subtracts a fixed number; fix any drift with trailing words."""
    have = word_count(text)
    if have > target:
        # Remove words from the end of the last prose paragraph.
        lines = text.rstrip("\n").split("\n")
        for idx in range(len(lines) - 1, -1, -1):
            ln = lines[idx]
            if ln and not ln[0] in (LINE, BLOCK_OPEN, BLOCK_CLOSE, "*", "#") and OPEN not in ln and "$$" not in ln:
                ws = ln.split(" ")
                cut = min(have - target, len(ws) - 1)
                lines[idx] = " ".join(ws[:-cut])
                have -= cut
                if have == target:
                    break
        text = "\n".join(lines) + "\n\n"
    elif have < target:
        text = text.rstrip("\n") + "\n\n" + gen.paragraph(target - have) + "\n\n"
    return text


def make_comments(gen: Gen, item: str, text: str, candidates, every: int, words: int, now: int):
    r = gen.r
    if not candidates:
        return []
    want = max(1, words // every)
    picks = r.sample(candidates, min(want, len(candidates)))
    chars = text
    out = []
    for start, end in sorted(picks):
        quote = chars[start:end]
        if not quote.strip():
            continue
        c = {
            "id": gen.new_id(),
            "item": item,
            "anchor": {"start": start, "end": end},
            "quote": quote,
            "before": chars[max(0, start - CONTEXT):start],
            "after": chars[end:end + CONTEXT],
            "body": r.choice(COMMENT_BODY),
            "created": now - r.randint(0, 90 * 86400),
            "status": "resolved" if r.random() < 0.3 else "open",
            "tag": r.choice(TAGS + [""]),
            "replies": [],
            "orphaned": False,
        }
        for _ in range(r.choice([0, 0, 0, 1, 2])):
            c["replies"].append({"id": gen.new_id(), "body": r.choice(REPLY_BODY), "created": c["created"] + r.randint(60, 86400)})
        out.append(c)
    return out


def build(target_words: int, out: Path, seed: int, avg_chapter: int, per_part: int, comment_every: int) -> dict:
    gen = Gen(seed + target_words)
    r = gen.r
    now = 1_760_000_000  # fixed so output is reproducible
    sizes, left = [], target_words
    while left > 0:
        n = min(left, max(300, int(avg_chapter * r.uniform(0.5, 1.5))))
        if left - n < 300:
            n = left
        sizes.append(n)
        left -= n

    files, comments, manuscript, part = {}, [], [], None
    for i, size in enumerate(sizes, 1):
        cid = gen.new_id()
        text, cand = gen.chapter(size)
        text = trim_to(text, cand, size, gen)
        # trimming may have shifted spans; keep only candidates whose quote is still plain text
        words = word_count(text)
        assert words == size, (words, size)
        files[f"chapters/{cid}.txt"] = text
        comments += make_comments(gen, cid, text, [(s, e) for s, e in cand if e <= len(text)], comment_every, size, now)
        node = {"type": "chapter", "id": cid, "title": f"Chapter {i}", "words": words}
        if (i - 1) % per_part == 0:
            part = {"type": "part", "id": gen.new_id(), "title": f"Part {(i - 1) // per_part + 1}", "children": []}
            manuscript.append(part)
        part["children"].append(node)

    pages = []
    for title in ("Characters", "Timeline", "Research"):
        pid = gen.new_id()
        body = "\n\n".join(gen.paragraph(r.randint(40, 120)) for _ in range(6)) + "\n"
        files[f"pages/{pid}.txt"] = body
        pages.append({"type": "page", "id": pid, "title": title, "words": word_count(body)})

    manifest = {
        "format": "justwrite", "format_version": 1, "app_version": "0.1.0", "id": gen.new_id(),
        "title": f"Benchmark {target_words:,} words", "created": now, "modified": now,
        "tree": {"manuscript": manuscript, "notes": pages},
        "note_tokens": {
            "line_note": "U+E000: starts a note that runs to the end of the line",
            "inline_open": "U+E001: opens a note that ends at U+E002 on the same line",
            "inline_close": "U+E002",
            "block_open": "U+E003: alone on a line, opens a block note",
            "block_close": "U+E004: alone on a line, closes it",
        },
    }
    out.mkdir(parents=True, exist_ok=True)
    label = f"{target_words // 1_000_000}m" if target_words % 1_000_000 == 0 else f"{target_words // 1000}k"
    path = out / f"bench_{label}.jw"
    stamp = (2026, 1, 1, 0, 0, 0)

    def put(z, name, data):
        info = zipfile.ZipInfo(name, stamp)
        info.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(info, data.encode("utf-8"))

    with zipfile.ZipFile(path, "w") as z:
        put(z, "manifest.json", json.dumps(manifest, indent=2, ensure_ascii=False))
        put(z, "book.json", json.dumps({"title": manifest["title"], "pen_name": "Bench Author"}, indent=2))
        put(z, "comments.json", json.dumps({"version": 1, "comments": comments}, indent=2, ensure_ascii=False))
        for name in sorted(files):
            put(z, name, files[name])
    verify(path, target_words)
    return {"path": path, "chapters": len(sizes), "parts": len(manuscript), "comments": len(comments),
            "mb": path.stat().st_size / 1e6, "raw_mb": sum(len(t.encode()) for t in files.values()) / 1e6}


def verify(path: Path, target: int):
    with zipfile.ZipFile(path) as z:
        assert z.testzip() is None
        manifest = json.loads(z.read("manifest.json"))
        total, leaves = 0, {}

        def walk(nodes):
            for n in nodes:
                if n["type"] == "part":
                    walk(n["children"])
                else:
                    leaves[n["id"]] = n

        walk(manifest["tree"]["manuscript"])
        for cid, n in leaves.items():
            text = z.read(f"chapters/{cid}.txt").decode("utf-8")
            assert word_count(text) == n["words"], cid
            total += n["words"]
        assert total == target, (total, target)
        for c in json.loads(z.read("comments.json"))["comments"]:
            text = z.read(f"chapters/{c['item']}.txt").decode("utf-8")
            assert text[c["anchor"]["start"]:c["anchor"]["end"]] == c["quote"], c["id"]


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--out", default="bench_data", help="output directory (default: bench_data, git-ignored)")
    p.add_argument("--sizes", type=int, nargs="+", default=[50_000, 100_000, 500_000, 1_000_000], help="word counts")
    p.add_argument("--seed", type=int, default=42)
    p.add_argument("--avg-chapter", type=int, default=3000, help="average words per chapter")
    p.add_argument("--chapters-per-part", type=int, default=12)
    p.add_argument("--comment-every", type=int, default=500, help="about one review comment per N words")
    a = p.parse_args()
    for size in a.sizes:
        t = time.time()
        s = build(size, Path(a.out), a.seed, a.avg_chapter, a.chapters_per_part, a.comment_every)
        print(f"{s['path']}: {size:>9,} words, {s['chapters']} chapters in {s['parts']} parts, "
              f"{s['comments']} comments, {s['raw_mb']:.1f} MB text -> {s['mb']:.1f} MB zip ({time.time() - t:.1f}s)")


if __name__ == "__main__":
    sys.exit(main())
