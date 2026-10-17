# PROMPT: Windows x86_64 CI failure 39039537533 — MSYS2 MINGW64 sox removal → CLANG64 migration (2026-10-17)

## Input
- User: "https://github.com/harrypm/FLAC-Chop/actions/runs/39039537533 - build failed"
- Run 39039537533 = "Build and release binary" (build.yml), triggered via workflow_dispatch
  ~2026-10-17T14:09Z from master @ d4f3722 (only the prompt-log commit since the
  green 2026-10-11 run on 59e8a65 — no workflow changes in between).

## Commands run and outputs
1. `gh run view 39039537533 --repo harrypm/FLAC-Chop`
   - Windows EXE (x86_64) job 117416491525 FAILED at "Setup MSYS2" in 30s:
     `Error: The process 'C:\Windows\system32\cmd.exe' failed with exit code 1`
   - Warning: "[msystem-mingw64] MINGW64 is deprecated. Migrate to UCRT64 or CLANG64."
   - All other jobs green or still running (Windows arm64 / macOS universal later
     completed green; final run conclusion: failure — only this job red).
2. Raw job log (saved to $env:TEMP\win-x86_64-job.log):
   - `pacman --noconfirm -S --needed ... 'git' 'mingw-w64-x86_64-gcc' ...`
   - `error: target not found: mingw-w64-x86_64-sox` → exit 1.
   - So the failure is BEFORE any build step: pacman cannot resolve the sox
     package name from the mingw64 repo DB.
3. packages.msys2.org (mingw-w64-sox base + binary package pages):
   - Page still listed mingw-w64-x86_64-sox (stale render) with file
     URL returning HTTP 200 — but that is NOT proof the DB still has it.
4. HARD DB VERIFICATION (downloaded live pacman DBs, tar -tf):
   - mingw64.db (repo.msys2.org AND mirror.msys2.org — identical):
     `mingw-w64-x86_64-sox` **MISSING**; libsoxr + python-soxr present;
     qt6-base 6.11.2-2, qt6-tools, gcc, cmake, ninja, pkgconf, flac, libogg,
     glslang, opencl-headers, opencl-icd, vulkan-headers, vulkan-loader all PRESENT.
   - ucrt64.db: `mingw-w64-ucrt-x86_64-sox-14.4.2.r3239.f3094754-2` PRESENT.
   - clang64.db: `mingw-w64-clang-x86_64-sox` + full workflow list PRESENT
     (clang instead of gcc; glslang/OpenCL/Vulkan all PRESENT — unlike clangarm64).
   - clangarm64.db: `mingw-w64-clang-aarch64-sox-14.4.2.r3239.f3094754-2` PRESENT
     (why the arm64 job kept working).
5. Upstream context (fetched):
   - MSYS2 news 2026-03-15: "Deprecating the MINGW64 Environment ... existing leaf
     packages may be removed ... please consider switching to UCRT64 or CLANG64."
   - msys2/MINGW-packages commits Oct 16-17, 2026: a wave of "...and drop mingw64"
     commits (xmake, python-send2trash, python-pipcl, elasticsearch, ...).
     => sox was dropped from the deprecated mingw64 repo between the green
     2026-10-11 run and 2026-10-17. NOT transient; rerunning cannot help.

## Diagnosis (evidence-verified)
The Windows EXE (x86_64) job installs `mingw-w64-x86_64-sox` from the deprecated
MSYS2 MINGW64 environment (build.yml:275-290 as-was). MSYS2 removed that package
from the mingw64 repo DB on 2026-10-16/17 during the MINGW64 phase-out, so
`pacman -S --needed` fails with "target not found" at the Setup MSYS2 step.

tests.yml (Tests workflow, runs on every push) installs the SAME removed package
(`mingw-w64-x86_64-sox`, msystem MINGW64) — it is green only because it last ran
before the drop; the next push would fail identically. Fixed in the same change.

## Fix: MINGW64 → CLANG64 (both workflows)
Why CLANG64 (not UCRT64): the arm64 job already pairs MSYS2 CLANGARM64 (llvm-mingw)
with Rust `aarch64-pc-windows-gnullvm` and is CI-green with full install/uninstall
packaging verification — the x86_64 job now mirrors that exact, proven pairing
(llvm-mingw on both sides, single UCRT, no mixed msvcrt/ucrt CRT in-process, which
the old "matching gcc" comment was written to avoid). CLANG64 also keeps
glslang/OpenCL/Vulkan, so the x86_64 ld-compress-ng accelerated backends stay ON
(the arm64 job passes -D...OFF only because clangARM64 lacks those packages).

### build.yml (windows-exe job)
- `msystem: MINGW64` → `CLANG64` (step renamed "Setup MSYS2 (CLANG64)"), with a
  comment recording the deprecation/removal evidence.
- Install list: `mingw-w64-x86_64-*` → `mingw-w64-clang-x86_64-*` (gcc→clang;
  all 14 entries verified PRESENT in the live clang64.db).
- Rust: `stable-x86_64-pc-windows-gnu` → `stable-x86_64-pc-windows-gnullvm`
  (verified shipped in rustup's stable channel manifest 2026-10-15); step renamed;
  comment mirrors the arm64 job's rationale (.a staticlib per gui/CMakeLists.txt).
- `cp /mingw64/bin/sox.exe` → `/clang64/bin/sox.exe` (windeployqt step).
- ldd fixpoint `BINDIR="/mingw64/bin"` → `"/clang64/bin"` + comment updates.
- gui/CMakeLists.txt audited beforehand: staticlib name gates on MSVC only
  (→ .a, fine for clang64/gnullvm); `-static*` link flags only apply for
  CMAKE_CXX_COMPILER_ID "GNU" (clang64 → shared runtime bundled by the ldd-walk,
  same as the green arm64 job); gomp only under FLAC_CHOP_STATIC_SOX (unused).
- Section comment updated: "Windows EXE (x86_64 via MSYS2 CLANG64)".

### tests.yml (cargo-test Windows leg)
- "Setup MSYS2 + SoX (Windows)": MINGW64 → CLANG64, install
  `mingw-w64-clang-x86_64-sox` (keeps "the exact build the Windows artifacts
  bundle" true, matching build.yml's new bundled sox).
- "Export the MSYS2 SoX path": `/mingw64/bin/sox(.exe)` → `/clang64/bin/sox(.exe)`.

### Left unchanged (documented decisions)
- build-static-deps.yml: one-off "run manually once" workflow (its tarballs are no
  longer used by build.yml); its packages (git/gcc/cmake/ninja/pkgconf/
  make/python/perl/tar/xz — no sox) are all still present in the live mingw64 DB.
  If it is ever re-run after further mingw64 pruning, migrate it the same way.
- dev_notes.md / docs / prompt_logs / deps/build-sox-static.sh mingw64 mentions:
  historical documentation, not CI-executable.

## Validation performed
- All 14 clang64 packages verified PRESENT in the live clang64.db (tar -tf).
- x86_64-pc-windows-gnullvm verified in the rustup stable channel TOML.
- Both edited YAMLs parse (yq) and the migrated fields asserted:
  build.yml windows-exe Setup with msystem CLANG64 + full clang64 install list,
  rust toolchain stable-x86_64-pc-windows-gnullvm, 12 steps intact, job keys
  intact; tests.yml sox setup CLANG64 + mingw-w64-clang-x86_64-sox.
- NOT YET VALIDATED: an actual CI run with the migrated job (build + bundle +
  installer + artifact checks on a real windows-2022 runner). CI must go green
  before this can be called fixed.

## Next steps (pending user)
- Commit + push the two workflow changes (+ this log).
- Re-dispatch "Build and release binary" (workflow_dispatch) and watch the
  Windows EXE (x86_64) leg; the next push also exercises the migrated tests.yml.
- Local dev note: the local MSYS2 (C:\msys64, mingw64 python on PATH) still has
  the old locally-installed packages, but any fresh local `pacman -Sy` + install
  of mingw64 sox will hit the same "target not found" — the local environment
  will eventually need the same CLANG64/UCRT64 migration if it's still used.
