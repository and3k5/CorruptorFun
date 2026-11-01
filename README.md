# CorruptorFun

A collection of browser toys that corrupt data and render the result, for fun.
It's a static Vue site deployed to GitHub Pages; there is no server, and files
you load never leave your browser.

## Collection

### BIOS corruptor (`#/bios`)

Load your own BIOS dump, corrupt it, and boot it in an emulator written for
this site (Rust compiled to WebAssembly). No BIOS images are included or
downloaded. Loaded BIOS files are kept in the browser's IndexedDB so you can
keep re-corrupting the untouched original.

- **PlayStation:** our own core in `emu/ps1` boots the real (corrupted) BIOS:
  intro, boot sound, and the memory card / CD player shell. Add your own game
  disc image (.cue + .bin, .bin or .iso) to see the licensed screen; emulation
  stops when the BIOS hands control to the game. The built-in test disc is in
  by default (Remove takes it out, and that is remembered). It is a tiny
  homemade disc (no game, no Sony data) to see the licensed screen without
  a game image; its demo waits for the boot sound to finish. The BIOS takes the license line and the 3D logo above
  "PlayStation" from the disc, so the test disc shows the license line and a
  homemade "CF" logo instead of Sony's. Its program is a little demo (a
  picture with an Amiga-style sine scroller), and with the test disc the
  emulator keeps running after the hand-off to show it. The demo's source is
  in `testdisc/`; `testdisc/build.sh` rebuilds `src/assets/testdisc.exe`
  (needs clang, ld.lld and Pillow).
- **Customize the boot** (PS1): change the SONY / COMPUTER / ENTERTAINMENT and
  "PlayStation" text, the license text and the logo on the licensed screen,
  and the boot sound samples (`src/lib/ps1custom/`). The text pictures are
  rebuilt from the letters cut out of your own BIOS (missing letters are
  drawn with a similar system font); the license text and the logo go on the
  test disc. A logo picture becomes pixel art made of lit flat triangles,
  the only kind the BIOS draws, within its limits (about 590 triangles,
  14 KB). Sounds in any format the browser can decode are re-encoded as SPU
  ADPCM into the room the original samples took (about 42 KB), at a lower
  rate if needed. Customizations are kept in IndexedDB, and corruptions start
  from the customized BIOS.
- **PlayStation 2:** in progress. You can already store, corrupt and download
  PS2 BIOS files.

**Easy corrupt** (PS1) boots a batch of random corruptions headlessly in
background workers (`src/emu/ps1-probe.worker.js`), samples what each one shows
and plays, and boots the best one that still has picture or sound and looks
different from the clean boot (`src/lib/easy.js`). The settings it picked land
in the Advanced form.

Advanced corruption engines: bit flip, random byte, nudge, XOR mask, smear,
block shift, zero out. Pick a byte range (or a preset region), a hit count or
"every Nth byte", and a seed.

On narrow screens you can turn on a mini screen that floats in the corner
while you scroll down to the options.

## Development

Needs Node 22 and Rust with the `wasm32-unknown-unknown` target
(`rustup target add wasm32-unknown-unknown`).

```sh
npm install
npm run dev     # builds the WASM core, then starts the dev server
npm test        # JS unit tests + Rust tests
npm run build   # static site in dist/
```

### PS1 core

`emu/ps1` is a PlayStation emulator scoped to the BIOS boot: R3000A CPU with
load/branch delay slots and exceptions, GTE, GPU software rasterizer, DMA,
timers, interrupt controller, CD-ROM controller, controller port and an SPU
mixer. It runs in a Web Worker (`src/emu/ps1.worker.js`), so a BIOS that
crashes or hangs can't freeze the page. The worker paces frames against the
wall clock (catching up by skipping pictures, not slowing down) and sends audio
straight to an AudioWorklet (`src/emu/ps1-audio.worklet.js`).

"Debug report" under the screen shows a timeline of CD-ROM commands, CPU
exceptions and periodic status, BIOS calls and unemulated I/O, ready to copy
into a bug report.

`emu/ps1/src/bin/cli.rs` is a headless runner for debugging (`--report`
prints the same report, `--wav file` saves the audio, `--trace N` prints N instructions and every I/O
access after the last frame, `--vram N` dumps VRAM at frame N, `--region A|E|I` sets the disc region):

```sh
cd emu && cargo run --release --bin ps1-cli -- BIOS.BIN --frames 600 --out frame.ppm
```

`emu/ps1/tests/mips` holds small PS1 test programs (CPU, GPU, CD-ROM) built
with clang's MIPS target: `./build.sh cputest.c`, then run the `.exe` with
`ps1-cli <any 512 KB file> --exe cputest.exe`.

### Adding a toy

Add an entry to `src/collection.js` and a route in `src/router.js`.

## Deployment

`.github/workflows/deploy.yml` builds and deploys on every push to `main`.
One-time setup: in the repository's **Settings → Pages**, set **Source** to
**GitHub Actions**.

Not affiliated with Sony.
