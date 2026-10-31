window.BENCHMARK_DATA = {
  "lastUpdate": 1793421510320,
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
      }
    ]
  }
}