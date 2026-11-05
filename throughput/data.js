window.BENCHMARK_DATA = {
  "lastUpdate": 1793853869552,
  "repoUrl": "https://github.com/0xdeafcafe/terminal-decline",
  "entries": {
    "Throughput": [
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "145de74b46b17d1ac209665b79667136111f8560",
          "message": "feat(app): restart into Terminal Decline.app once the old bundle is gone\n\nA dogfood build found its bundle again from its executable when the\nexecutable changed. Once the bundle is removed rather than replaced, as\nthe dogfood script will do with Decline.app when the app is renamed,\nthat fails and the build quietly stays running with its windows.\n\nKeep the bundle path from startup, and when it is gone, restart into\n\"Terminal Decline.app\" in the same folder with the saved windows. Land\nthis before the rename so the running copy carries its windows over.\n\nCo-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-10-29T03:56:59Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/145de74b46b17d1ac209665b79667136111f8560"
        },
        "date": 1793247587226,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "parse/coloured",
            "value": 179.61,
            "unit": "MiB/s"
          },
          {
            "name": "parse/plain",
            "value": 461.98,
            "unit": "MiB/s"
          },
          {
            "name": "parse/seq",
            "value": 112.43,
            "unit": "MiB/s"
          },
          {
            "name": "parse/tui",
            "value": 302.08,
            "unit": "MiB/s"
          },
          {
            "name": "parse/unicode",
            "value": 137.15,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/coloured",
            "value": 181.79,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/plain",
            "value": 456.59,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/seq",
            "value": 115.57,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/tui",
            "value": 282.87,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/unicode",
            "value": 120.04,
            "unit": "MiB/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "cc0828b1d7565ebdcdf9f09ca8ee17f48c97199f",
          "message": "test(app): list iTerm2 and Sixel images in the dump smoke test\n\nWith images = true, an iTerm2 image and a Sixel image printed by the\nprogram must both be counted and ready in the dump's images section,\nwith some text drawn too. Not run locally: CI runs the smoke tests.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-10-29T19:23:49Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/cc0828b1d7565ebdcdf9f09ca8ee17f48c97199f"
        },
        "date": 1793335302131,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "parse/clusters",
            "value": 57.81,
            "unit": "MiB/s"
          },
          {
            "name": "parse/coloured",
            "value": 112.25,
            "unit": "MiB/s"
          },
          {
            "name": "parse/plain",
            "value": 335.6,
            "unit": "MiB/s"
          },
          {
            "name": "parse/seq",
            "value": 97.17,
            "unit": "MiB/s"
          },
          {
            "name": "parse/tui",
            "value": 166.87,
            "unit": "MiB/s"
          },
          {
            "name": "parse/unicode",
            "value": 73.61,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/clusters",
            "value": 79.69,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/coloured",
            "value": 136.21,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/plain",
            "value": 337.13,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/seq",
            "value": 99.92,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/tui",
            "value": 227.11,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/unicode",
            "value": 85.68,
            "unit": "MiB/s"
          },
          {
            "name": "history decode/background",
            "value": 4735210,
            "unit": "elem/s"
          },
          {
            "name": "history decode/cjk",
            "value": 3403609,
            "unit": "elem/s"
          },
          {
            "name": "history decode/emoji",
            "value": 2768239,
            "unit": "elem/s"
          },
          {
            "name": "history decode/gradient",
            "value": 382707,
            "unit": "elem/s"
          },
          {
            "name": "history decode/ls",
            "value": 4629779,
            "unit": "elem/s"
          },
          {
            "name": "history decode/plain",
            "value": 8513388,
            "unit": "elem/s"
          },
          {
            "name": "history push/background",
            "value": 3608568,
            "unit": "elem/s"
          },
          {
            "name": "history push/cjk",
            "value": 5575603,
            "unit": "elem/s"
          },
          {
            "name": "history push/emoji",
            "value": 5972577,
            "unit": "elem/s"
          },
          {
            "name": "history push/gradient",
            "value": 278357,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/background",
            "value": 3613464,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 5452079,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 6816349,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 327564,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/ls",
            "value": 5712222,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/plain",
            "value": 19598384,
            "unit": "elem/s"
          },
          {
            "name": "history push/ls",
            "value": 5511629,
            "unit": "elem/s"
          },
          {
            "name": "history push/plain",
            "value": 20614715,
            "unit": "elem/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "f72c550128f9a3e7b7089988b6723b0cfa047fba",
          "message": "perf(render): let a hidden tab's every shown frame be purged\n\nA hidden tab marked the surfaces of its last two frames purgeable, but\nthe window server doesn't always keep the last one presented: a tab\nbehind others (and a window in the background test mode) may keep an\nolder one, often the window's first frame, which stayed counted in\nfootprint for as long as the window lived, 6.5 MB at 1624x1018.\n\nThe renderer now keeps the surface of each drawable the layer may hold\n(three, newest first) and marks them all purgeable when the window is\nhidden, and again 0.5 s and 3 s later, for a frame presented just\nbefore the hide (a tab switch that drew once more). Surfaces of another\nsize are let go as soon as a frame of the new size is presented, since\nthe layer never hands them out again. Under the test control a hidden\nwindow still draws (at 1x1); it is hidden again after that frame, and\nthe 1x1 frame leaves the full-size surfaces purgeable, so the test\ncontrol no longer undoes the hide.\n\nMeasured: release, background test mode, temp XDG_CONFIG_HOME, Menlo\n13, 100x30, footprint 40 s after the last frame, interleaved with\norigin/main, load 3-19:\n- 4 tabs: 44.72 / 44.61 / 42.89 / 44.88 / 44.88 MB before,\n  38.64 / 38.13 / 40.56 / 38.22 / 37.08 after; the window-sized\n  image (owned, unmapped, 6.47 MB) is gone in every run. The rest of\n  the spread is a 2 MB GPU driver allocation that comes and goes in\n  both.\n- 2 tabs, switched back to the first: 42.16 / 40.16 / 42.59 before,\n  35.78 / 38.69 / 35.69 after.\n- 1 tab: 28.39 / 28.47 / 28.36 before, 26.95 / 26.94 / 27.17 after\n  (the tab bar change before this one).\nA shown tab's first frame is still committed 1-3 ms after the tab is\nselected.\n\nCo-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-10-31T02:59:22Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/f72c550128f9a3e7b7089988b6723b0cfa047fba"
        },
        "date": 1793421509631,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "parse/clusters",
            "value": 84.91,
            "unit": "MiB/s"
          },
          {
            "name": "parse/coloured",
            "value": 201.36,
            "unit": "MiB/s"
          },
          {
            "name": "parse/plain",
            "value": 483.77,
            "unit": "MiB/s"
          },
          {
            "name": "parse/seq",
            "value": 130.65,
            "unit": "MiB/s"
          },
          {
            "name": "parse/tui",
            "value": 327.88,
            "unit": "MiB/s"
          },
          {
            "name": "parse/unicode",
            "value": 127.6,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/clusters",
            "value": 88.38,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/coloured",
            "value": 202.38,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/plain",
            "value": 555.01,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/seq",
            "value": 133.81,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/tui",
            "value": 335.36,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/unicode",
            "value": 128.51,
            "unit": "MiB/s"
          },
          {
            "name": "history decode/background",
            "value": 8233765,
            "unit": "elem/s"
          },
          {
            "name": "history decode/cjk",
            "value": 5604425,
            "unit": "elem/s"
          },
          {
            "name": "history decode/emoji",
            "value": 3925573,
            "unit": "elem/s"
          },
          {
            "name": "history decode/gradient",
            "value": 456885,
            "unit": "elem/s"
          },
          {
            "name": "history decode/ls",
            "value": 5790939,
            "unit": "elem/s"
          },
          {
            "name": "history decode/plain",
            "value": 10157052,
            "unit": "elem/s"
          },
          {
            "name": "history push/background",
            "value": 4816280,
            "unit": "elem/s"
          },
          {
            "name": "history push/cjk",
            "value": 6579460,
            "unit": "elem/s"
          },
          {
            "name": "history push/emoji",
            "value": 8376383,
            "unit": "elem/s"
          },
          {
            "name": "history push/gradient",
            "value": 341354,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/background",
            "value": 5409023,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 6204322,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 8923690,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 348143,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/ls",
            "value": 11818278,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/plain",
            "value": 23432982,
            "unit": "elem/s"
          },
          {
            "name": "history push/ls",
            "value": 9930335,
            "unit": "elem/s"
          },
          {
            "name": "history push/plain",
            "value": 24322502,
            "unit": "elem/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "b14aa9593c843d56217f98a6331b004edfea91fc",
          "message": "docs(design): note the glyph thread under R.1\n\nHow fonts share their registry with the glyph thread, which glyphs\nstay synchronous, how the thread wakes the app, why bitmaps are put in\nthe textures only when drawn and unwanted glyphs skipped, the numbers,\nand what is left: cluster and ligature shaping, and the upload's wait\nfor the GPU.\n\nCo-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-10-31T18:29:36Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/b14aa9593c843d56217f98a6331b004edfea91fc"
        },
        "date": 1793507905680,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "parse/clusters",
            "value": 94.56,
            "unit": "MiB/s"
          },
          {
            "name": "parse/coloured",
            "value": 239.43,
            "unit": "MiB/s"
          },
          {
            "name": "parse/plain",
            "value": 577.32,
            "unit": "MiB/s"
          },
          {
            "name": "parse/seq",
            "value": 141.81,
            "unit": "MiB/s"
          },
          {
            "name": "parse/tui",
            "value": 389.37,
            "unit": "MiB/s"
          },
          {
            "name": "parse/unicode",
            "value": 153.82,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/clusters",
            "value": 80.94,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/coloured",
            "value": 243.73,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/plain",
            "value": 567.14,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/seq",
            "value": 149.92,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/tui",
            "value": 374.56,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/unicode",
            "value": 145.61,
            "unit": "MiB/s"
          },
          {
            "name": "history decode/background",
            "value": 12324061,
            "unit": "elem/s"
          },
          {
            "name": "history decode/cjk",
            "value": 6922163,
            "unit": "elem/s"
          },
          {
            "name": "history decode/emoji",
            "value": 4896761,
            "unit": "elem/s"
          },
          {
            "name": "history decode/gradient",
            "value": 592203,
            "unit": "elem/s"
          },
          {
            "name": "history decode/ls",
            "value": 7873547,
            "unit": "elem/s"
          },
          {
            "name": "history decode/plain",
            "value": 14620570,
            "unit": "elem/s"
          },
          {
            "name": "history push/background",
            "value": 6784184,
            "unit": "elem/s"
          },
          {
            "name": "history push/cjk",
            "value": 8782742,
            "unit": "elem/s"
          },
          {
            "name": "history push/emoji",
            "value": 10043184,
            "unit": "elem/s"
          },
          {
            "name": "history push/gradient",
            "value": 434600,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/background",
            "value": 6761444,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 8457571,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 9188649,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 412817,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/ls",
            "value": 13394168,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/plain",
            "value": 29421260,
            "unit": "elem/s"
          },
          {
            "name": "history push/ls",
            "value": 10817363,
            "unit": "elem/s"
          },
          {
            "name": "history push/plain",
            "value": 29504569,
            "unit": "elem/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "81463a2495d70eed38a537c4ada16bcc4a6d2af4",
          "message": "feat(restore): reopen the windows after any quit\n\nQuitting now always saves the window list (places, tabs, panes, full\nscreen) and the next launch reopens it. History still follows\nrestore_scrollback: saved on quit only with \"always\", never while\nSecure Keyboard Entry is on. Before, a plain quit with the default\nsettings forgot every window and the next launch opened one in the\nmiddle of the screen.",
          "timestamp": "2026-11-02T03:20:23Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/81463a2495d70eed38a537c4ada16bcc4a6d2af4"
        },
        "date": 1793594456563,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "parse/clusters",
            "value": 109.16,
            "unit": "MiB/s"
          },
          {
            "name": "parse/coloured",
            "value": 248.08,
            "unit": "MiB/s"
          },
          {
            "name": "parse/plain",
            "value": 726.14,
            "unit": "MiB/s"
          },
          {
            "name": "parse/seq",
            "value": 215.23,
            "unit": "MiB/s"
          },
          {
            "name": "parse/tui",
            "value": 391.78,
            "unit": "MiB/s"
          },
          {
            "name": "parse/unicode",
            "value": 141.5,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/clusters",
            "value": 100.09,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/coloured",
            "value": 257.79,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/plain",
            "value": 670.27,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/seq",
            "value": 195.14,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/tui",
            "value": 379.79,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/unicode",
            "value": 141.68,
            "unit": "MiB/s"
          },
          {
            "name": "history decode/background",
            "value": 13290466,
            "unit": "elem/s"
          },
          {
            "name": "history decode/cjk",
            "value": 6606579,
            "unit": "elem/s"
          },
          {
            "name": "history decode/emoji",
            "value": 5001253,
            "unit": "elem/s"
          },
          {
            "name": "history decode/gradient",
            "value": 576024,
            "unit": "elem/s"
          },
          {
            "name": "history decode/ls",
            "value": 7772482,
            "unit": "elem/s"
          },
          {
            "name": "history decode/plain",
            "value": 14015322,
            "unit": "elem/s"
          },
          {
            "name": "history push/background",
            "value": 6644708,
            "unit": "elem/s"
          },
          {
            "name": "history push/cjk",
            "value": 7674447,
            "unit": "elem/s"
          },
          {
            "name": "history push/emoji",
            "value": 9923449,
            "unit": "elem/s"
          },
          {
            "name": "history push/gradient",
            "value": 407855,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/background",
            "value": 6660678,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 8449215,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 9406682,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 419876,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/ls",
            "value": 14047249,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/plain",
            "value": 31928594,
            "unit": "elem/s"
          },
          {
            "name": "history push/ls",
            "value": 11139546,
            "unit": "elem/s"
          },
          {
            "name": "history push/plain",
            "value": 32203392,
            "unit": "elem/s"
          },
          {
            "name": "typing/echo",
            "value": 20164903,
            "unit": "elem/s"
          },
          {
            "name": "typing/echo+report",
            "value": 2001407,
            "unit": "elem/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "41fbb01b4db6a47145d117566552277dd563b48d",
          "message": "perf(core): reserve half the last history block's size, not all of it\n\n50ca009 started each open block as big as the last one ended, which\nleft the open block's spare room at half a block on average: scrollback\nbytes per line rose past the nightly limit for CJK (69 to 76, limit 75)\nand plain text (44 to 49). Reserving half keeps it to one reallocation\nper buffer instead of a dozen: CJK 72, plain 46, every kind under its\nlimit. parse_1k/plain, interleaved runs at load 6-10: 7.39 and 7.07 ms\nagainst 7.06 and 7.22 ms with the whole size, within the noise.",
          "timestamp": "2026-11-02T09:32:04Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/41fbb01b4db6a47145d117566552277dd563b48d"
        },
        "date": 1793614386001,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "parse/clusters",
            "value": 89.45,
            "unit": "MiB/s"
          },
          {
            "name": "parse/coloured",
            "value": 177.29,
            "unit": "MiB/s"
          },
          {
            "name": "parse/plain",
            "value": 577.97,
            "unit": "MiB/s"
          },
          {
            "name": "parse/seq",
            "value": 148.4,
            "unit": "MiB/s"
          },
          {
            "name": "parse/tui",
            "value": 267.28,
            "unit": "MiB/s"
          },
          {
            "name": "parse/unicode",
            "value": 122.77,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/clusters",
            "value": 86.14,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/coloured",
            "value": 170.11,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/plain",
            "value": 567.25,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/seq",
            "value": 152.47,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/tui",
            "value": 246.47,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/unicode",
            "value": 124.4,
            "unit": "MiB/s"
          },
          {
            "name": "history decode/background",
            "value": 4438237,
            "unit": "elem/s"
          },
          {
            "name": "history decode/cjk",
            "value": 4992029,
            "unit": "elem/s"
          },
          {
            "name": "history decode/emoji",
            "value": 3354505,
            "unit": "elem/s"
          },
          {
            "name": "history decode/gradient",
            "value": 552435,
            "unit": "elem/s"
          },
          {
            "name": "history decode/ls",
            "value": 4264299,
            "unit": "elem/s"
          },
          {
            "name": "history decode/plain",
            "value": 7011100,
            "unit": "elem/s"
          },
          {
            "name": "history push/background",
            "value": 4476993,
            "unit": "elem/s"
          },
          {
            "name": "history push/cjk",
            "value": 6170419,
            "unit": "elem/s"
          },
          {
            "name": "history push/emoji",
            "value": 7398656,
            "unit": "elem/s"
          },
          {
            "name": "history push/gradient",
            "value": 274862,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/background",
            "value": 4799146,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 7089565,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 8215337,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 290071,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/ls",
            "value": 8015740,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/plain",
            "value": 23239888,
            "unit": "elem/s"
          },
          {
            "name": "history push/ls",
            "value": 8018152,
            "unit": "elem/s"
          },
          {
            "name": "history push/plain",
            "value": 21491272,
            "unit": "elem/s"
          },
          {
            "name": "typing/echo",
            "value": 17194446,
            "unit": "elem/s"
          },
          {
            "name": "typing/echo+report",
            "value": 1961315,
            "unit": "elem/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "a053e5b334cdd0edc28531d4fe81aad77d28f6cd",
          "message": "fix(complete): no command line above a pane's first prompt\n\nThe faint line marks one command off from the one before it, but it\nwas drawn above the first prompt too whenever that wasn't on the top\nrow: under the \"restored from …\" or welcome line it only underlined\nthat line. The first prompt has no command above it, so it gets none.",
          "timestamp": "2026-11-02T12:26:38Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/a053e5b334cdd0edc28531d4fe81aad77d28f6cd"
        },
        "date": 1793680962588,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "parse/clusters",
            "value": 55.19,
            "unit": "MiB/s"
          },
          {
            "name": "parse/coloured",
            "value": 205.39,
            "unit": "MiB/s"
          },
          {
            "name": "parse/plain",
            "value": 559.85,
            "unit": "MiB/s"
          },
          {
            "name": "parse/seq",
            "value": 157.04,
            "unit": "MiB/s"
          },
          {
            "name": "parse/tui",
            "value": 313.77,
            "unit": "MiB/s"
          },
          {
            "name": "parse/unicode",
            "value": 111.3,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/clusters",
            "value": 81.43,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/coloured",
            "value": 194.54,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/plain",
            "value": 411.03,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/seq",
            "value": 162.48,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/tui",
            "value": 307.06,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/unicode",
            "value": 118.44,
            "unit": "MiB/s"
          },
          {
            "name": "history decode/background",
            "value": 7066009,
            "unit": "elem/s"
          },
          {
            "name": "history decode/cjk",
            "value": 3749777,
            "unit": "elem/s"
          },
          {
            "name": "history decode/emoji",
            "value": 2698334,
            "unit": "elem/s"
          },
          {
            "name": "history decode/gradient",
            "value": 474622,
            "unit": "elem/s"
          },
          {
            "name": "history decode/ls",
            "value": 5053923,
            "unit": "elem/s"
          },
          {
            "name": "history decode/plain",
            "value": 10244338,
            "unit": "elem/s"
          },
          {
            "name": "history push/background",
            "value": 4829030,
            "unit": "elem/s"
          },
          {
            "name": "history push/cjk",
            "value": 6556715,
            "unit": "elem/s"
          },
          {
            "name": "history push/emoji",
            "value": 8869266,
            "unit": "elem/s"
          },
          {
            "name": "history push/gradient",
            "value": 265322,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/background",
            "value": 5413714,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 6045829,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 8422553,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 360364,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/ls",
            "value": 9345712,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/plain",
            "value": 19326072,
            "unit": "elem/s"
          },
          {
            "name": "history push/ls",
            "value": 8158274,
            "unit": "elem/s"
          },
          {
            "name": "history push/plain",
            "value": 18965236,
            "unit": "elem/s"
          },
          {
            "name": "typing/echo",
            "value": 17626428,
            "unit": "elem/s"
          },
          {
            "name": "typing/echo+report",
            "value": 1785627,
            "unit": "elem/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "394c9c9a20d04261bfee6e3e313c160fa014fe21",
          "message": "build(deps): bump miniz_oxide from 0.8.9 to 0.9.1 (#13)\n\nBumps [miniz_oxide](https://github.com/Frommi/miniz_oxide) from 0.8.9 to 0.9.1.\n- [Changelog](https://github.com/Frommi/miniz_oxide/blob/master/CHANGELOG.md)\n- [Commits](https://github.com/Frommi/miniz_oxide/commits)\n\n---\nupdated-dependencies:\n- dependency-name: miniz_oxide\n  dependency-version: 0.9.1\n  dependency-type: direct:production\n  update-type: version-update:semver-minor\n...\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-11-04T20:50:02Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/394c9c9a20d04261bfee6e3e313c160fa014fe21"
        },
        "date": 1793853868787,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "parse/clusters",
            "value": 82.8,
            "unit": "MiB/s"
          },
          {
            "name": "parse/coloured",
            "value": 189.41,
            "unit": "MiB/s"
          },
          {
            "name": "parse/plain",
            "value": 438.36,
            "unit": "MiB/s"
          },
          {
            "name": "parse/seq",
            "value": 153.16,
            "unit": "MiB/s"
          },
          {
            "name": "parse/tui",
            "value": 300.71,
            "unit": "MiB/s"
          },
          {
            "name": "parse/unicode",
            "value": 118.86,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/clusters",
            "value": 83.31,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/coloured",
            "value": 186.55,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/plain",
            "value": 448.79,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/seq",
            "value": 156.39,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/tui",
            "value": 297.53,
            "unit": "MiB/s"
          },
          {
            "name": "parse_1k/unicode",
            "value": 115.66,
            "unit": "MiB/s"
          },
          {
            "name": "history decode/background",
            "value": 7846373,
            "unit": "elem/s"
          },
          {
            "name": "history decode/cjk",
            "value": 4520679,
            "unit": "elem/s"
          },
          {
            "name": "history decode/emoji",
            "value": 3605852,
            "unit": "elem/s"
          },
          {
            "name": "history decode/gradient",
            "value": 481709,
            "unit": "elem/s"
          },
          {
            "name": "history decode/ls",
            "value": 5347100,
            "unit": "elem/s"
          },
          {
            "name": "history decode/plain",
            "value": 8374255,
            "unit": "elem/s"
          },
          {
            "name": "history push/background",
            "value": 4693437,
            "unit": "elem/s"
          },
          {
            "name": "history push/cjk",
            "value": 7222585,
            "unit": "elem/s"
          },
          {
            "name": "history push/emoji",
            "value": 6435865,
            "unit": "elem/s"
          },
          {
            "name": "history push/gradient",
            "value": 324643,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/background",
            "value": 5387234,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 6540597,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 7483375,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 333718,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/ls",
            "value": 11825966,
            "unit": "elem/s"
          },
          {
            "name": "history push/keyed/plain",
            "value": 24105983,
            "unit": "elem/s"
          },
          {
            "name": "history push/ls",
            "value": 7349032,
            "unit": "elem/s"
          },
          {
            "name": "history push/plain",
            "value": 22467082,
            "unit": "elem/s"
          },
          {
            "name": "typing/echo",
            "value": 14992798,
            "unit": "elem/s"
          },
          {
            "name": "typing/echo+report",
            "value": 1695159,
            "unit": "elem/s"
          }
        ]
      }
    ]
  }
}