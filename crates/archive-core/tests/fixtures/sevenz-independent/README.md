# Independent Solid Fixtures

Generated locally on 2026-10-05 with p7zip 17.05 (7-Zip 17.05).
Inputs are repository-authored ASCII text: `first.txt` contains
`first payload` plus LF; `last.txt` contains `last payload` plus LF.
Both files share one solid folder. No upstream writer is used.

Commands, run in the directory containing these two inputs:

```sh
7z a -t7z -m0=Copy -ms=on solid-copy.7z first.txt last.txt
7z a -t7z -m0=LZMA -ms=on solid-lzma.7z first.txt last.txt
7z a -t7z -m0=LZMA2 -ms=on solid-lzma2.7z first.txt last.txt
7z a -t7z -m0=BCJ -m1=LZMA2 -ms=on solid-bcj-lzma2.7z first.txt last.txt
```

Timestamps are incidental; tests assert decoded payload and folder behavior,
not byte-identical regeneration.
