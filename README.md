# CorruptorFun

A collection of browser toys that corrupt data and render the result, for fun.
It's a static Vue site deployed to GitHub Pages; there is no server, and files
you load never leave your browser.

## Collection

### BIOS corruptor (`#/bios`)

Load your own BIOS dump, corrupt it, and watch the boot sequence break.
No BIOS images are included or downloaded: you have to supply your own.
Loaded files are kept in the browser's IndexedDB so you can keep re-corrupting
the untouched original without picking it again.

| Console | How it's rendered |
| --- | --- |
| PlayStation | **Real BIOS** in an emulator (EmulatorJS + pcsx_rearmed), plus a simulated mode. With no disc, the BIOS plays its intro and opens the shell; load your own game image to see the licensed screen. |
| PlayStation 2 | Simulated: towers, SCE screen, then the PS2 logo (licensed disc) or the browser menu. |
| Xbox | Simulated: green energy swirl into the X emblem. |
| Xbox 360 | Simulated: the green orb. Uses a NAND dump (or any file) as the data source. |

**Why not emulate them all?** There's no usable in-browser PS2, Xbox or 360
emulator, and the Xbox / 360 boot code is encrypted. The simulated scenes are
re-creations where each visual parameter is wired to its own slice of your
file: clean bytes give the stock animation, corrupted bytes in that slice bend
that parameter. So *where* you corrupt decides *what* breaks, and a seed always
gives the same result. You can download the corrupted BIOS and run it in a real
emulator (PCSX2, xemu, ...).

Corruption engines: bit flip, random byte, nudge, XOR mask, smear, block shift,
zero out. Pick a byte range (or a preset region), a hit count or "every Nth
byte", and a seed.

## Development

```sh
npm install
npm run dev     # local dev server
npm test        # unit tests for the corruption engine
npm run build   # static site in dist/
```

To test the PS1 emulator offline, put the `@emulatorjs/emulatorjs` package's
`data/` folder at `public/ejs-local/emulatorjs/data/` and the
`@emulatorjs/core-pcsx_rearmed` package at `public/ejs-local/core-pcsx_rearmed/`,
then run `VITE_EJS_LOCAL=/ejs-local/ npm run dev`. Don't commit that folder.

### Adding a toy

Add an entry to `src/collection.js` and a route in `src/router.js`.

## Deployment

`.github/workflows/deploy.yml` builds and deploys on every push to `main`.
One-time setup: in the repository's **Settings → Pages**, set **Source** to
**GitHub Actions**.

## Credits

The PS1 emulator is [EmulatorJS](https://github.com/EmulatorJS/EmulatorJS) with
the [pcsx_rearmed](https://github.com/libretro/pcsx_rearmed) core (both GPL-3.0),
loaded from jsDelivr at runtime. Not affiliated with Sony or Microsoft.
