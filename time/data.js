window.BENCHMARK_DATA = {
  "lastUpdate": 1794112903540,
  "repoUrl": "https://github.com/0xdeafcafe/terminal-decline",
  "entries": {
    "Time per iteration": [
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
        "date": 1793247590272,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "frame 300x100 coloured",
            "value": 111.698,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain",
            "value": 103.965,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain",
            "value": 6.55,
            "unit": "µs"
          },
          {
            "name": "parse/coloured",
            "value": 22271.136,
            "unit": "µs"
          },
          {
            "name": "parse/plain",
            "value": 8658.418,
            "unit": "µs"
          },
          {
            "name": "parse/seq",
            "value": 35577.179,
            "unit": "µs"
          },
          {
            "name": "parse/tui",
            "value": 13241.571,
            "unit": "µs"
          },
          {
            "name": "parse/unicode",
            "value": 29164.514,
            "unit": "µs"
          },
          {
            "name": "parse_1k/coloured",
            "value": 22003.935,
            "unit": "µs"
          },
          {
            "name": "parse_1k/plain",
            "value": 8760.672,
            "unit": "µs"
          },
          {
            "name": "parse_1k/seq",
            "value": 34612.608,
            "unit": "µs"
          },
          {
            "name": "parse_1k/tui",
            "value": 14140.741,
            "unit": "µs"
          },
          {
            "name": "parse_1k/unicode",
            "value": 33322.733,
            "unit": "µs"
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
        "date": 1793335306008,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "frame 1 pane 160x48 coloured",
            "value": 43.275,
            "unit": "µs"
          },
          {
            "name": "frame 1 pane 160x48 coloured unchanged",
            "value": 7.522,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured",
            "value": 165.716,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history",
            "value": 228.519,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured live",
            "value": 143.856,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row",
            "value": 41.151,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row live",
            "value": 148.648,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled",
            "value": 52.75,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled live",
            "value": 153.716,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured unchanged",
            "value": 37.443,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style",
            "value": 201.505,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style live",
            "value": 192.471,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style unchanged",
            "value": 30.106,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain",
            "value": 152.052,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history",
            "value": 157.597,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain live",
            "value": 152.339,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain unchanged",
            "value": 41.168,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode",
            "value": 164.9,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode history",
            "value": 452.702,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode live",
            "value": 183.913,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo",
            "value": 298.55,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo unchanged",
            "value": 19.713,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode unchanged",
            "value": 22.273,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured",
            "value": 49.181,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured unchanged",
            "value": 11.489,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain",
            "value": 11.123,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain live",
            "value": 10.992,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row",
            "value": 2.66,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row live",
            "value": 10.388,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled",
            "value": 4.541,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled live",
            "value": 10.245,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain unchanged",
            "value": 2.216,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo",
            "value": 20.221,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo unchanged",
            "value": 2.004,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images 10 placements",
            "value": 2.851,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images none",
            "value": 2.159,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images placeholders",
            "value": 2.199,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains",
            "value": 74.114,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains cold",
            "value": 3223.844,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains one row",
            "value": 5.242,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains rebuilt",
            "value": 37.865,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains scrolled",
            "value": 7.495,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains unchanged",
            "value": 3.292,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo",
            "value": 18.827,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo cold",
            "value": 19.97,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo one row",
            "value": 3.333,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo rebuilt",
            "value": 18.265,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo scrolled",
            "value": 6.058,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo unchanged",
            "value": 3.222,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains",
            "value": 606.966,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains cold",
            "value": 4428,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains one row",
            "value": 28.742,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains rebuilt",
            "value": 311.529,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains scrolled",
            "value": 45.585,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains unchanged",
            "value": 23.231,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo",
            "value": 181.584,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo cold",
            "value": 193.278,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo one row",
            "value": 24.692,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo rebuilt",
            "value": 194.303,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo scrolled",
            "value": 41.638,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo unchanged",
            "value": 27.887,
            "unit": "µs"
          },
          {
            "name": "history decode/background",
            "value": 2111.839,
            "unit": "µs"
          },
          {
            "name": "history decode/cjk",
            "value": 2938.058,
            "unit": "µs"
          },
          {
            "name": "history decode/emoji",
            "value": 3612.405,
            "unit": "µs"
          },
          {
            "name": "history decode/gradient",
            "value": 26129.62,
            "unit": "µs"
          },
          {
            "name": "history decode/ls",
            "value": 2159.93,
            "unit": "µs"
          },
          {
            "name": "history decode/plain",
            "value": 1174.62,
            "unit": "µs"
          },
          {
            "name": "history push/background",
            "value": 2771.183,
            "unit": "µs"
          },
          {
            "name": "history push/cjk",
            "value": 1793.528,
            "unit": "µs"
          },
          {
            "name": "history push/emoji",
            "value": 1674.319,
            "unit": "µs"
          },
          {
            "name": "history push/gradient",
            "value": 35925.14,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/background",
            "value": 2767.428,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 1834.163,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 1467.061,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 30528.413,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/ls",
            "value": 1750.632,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/plain",
            "value": 510.246,
            "unit": "µs"
          },
          {
            "name": "history push/ls",
            "value": 1814.346,
            "unit": "µs"
          },
          {
            "name": "history push/plain",
            "value": 485.09,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_200",
            "value": 24.809,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_300",
            "value": 13.734,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_200",
            "value": 133.047,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_300",
            "value": 56.04,
            "unit": "µs"
          },
          {
            "name": "parse/clusters",
            "value": 69191.18,
            "unit": "µs"
          },
          {
            "name": "parse/coloured",
            "value": 35634.915,
            "unit": "µs"
          },
          {
            "name": "parse/plain",
            "value": 11919.178,
            "unit": "µs"
          },
          {
            "name": "parse/seq",
            "value": 41166.878,
            "unit": "µs"
          },
          {
            "name": "parse/tui",
            "value": 23970.725,
            "unit": "µs"
          },
          {
            "name": "parse/unicode",
            "value": 54341.106,
            "unit": "µs"
          },
          {
            "name": "parse_1k/clusters",
            "value": 50192.079,
            "unit": "µs"
          },
          {
            "name": "parse_1k/coloured",
            "value": 29367.94,
            "unit": "µs"
          },
          {
            "name": "parse_1k/plain",
            "value": 11864.836,
            "unit": "µs"
          },
          {
            "name": "parse_1k/seq",
            "value": 40032.339,
            "unit": "µs"
          },
          {
            "name": "parse_1k/tui",
            "value": 17612.801,
            "unit": "µs"
          },
          {
            "name": "parse_1k/unicode",
            "value": 46683.237,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode",
            "value": 47.768,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare",
            "value": 60.372,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare unchanged",
            "value": 6.55,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build",
            "value": 72.361,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build unchanged",
            "value": 11.497,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 encode",
            "value": 66.005,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_ascii",
            "value": 331.655,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_cjk",
            "value": 280.015,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_ascii",
            "value": 336.221,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_cjk",
            "value": 263.575,
            "unit": "µs"
          },
          {
            "name": "search_100k/filters/build_all",
            "value": 11269.551,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent",
            "value": 79.65,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent_ignore_case",
            "value": 53.279,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/common_ignore_case",
            "value": 6842.955,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/non_ascii_ignore_case",
            "value": 124.321,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare",
            "value": 55.429,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare_ignore_case",
            "value": 85.501,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent",
            "value": 640.522,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent_ignore_case",
            "value": 765.54,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/common_ignore_case",
            "value": 6866.705,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/non_ascii_ignore_case",
            "value": 671.341,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare",
            "value": 654.003,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare_ignore_case",
            "value": 939.027,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent",
            "value": 35.349,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent_ignore_case",
            "value": 31.506,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/common_ignore_case",
            "value": 35.586,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/non_ascii_ignore_case",
            "value": 28.249,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare",
            "value": 29.382,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare_ignore_case",
            "value": 29.737,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 full copy",
            "value": 10.999,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 history full copy",
            "value": 73.881,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 spinner",
            "value": 6.169,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 unchanged",
            "value": 5.351,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 full copy",
            "value": 1.228,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 spinner",
            "value": 1.617,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 unchanged",
            "value": 1.579,
            "unit": "µs"
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
        "date": 1793421512148,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "frame 1 pane 160x48 coloured",
            "value": 42.699,
            "unit": "µs"
          },
          {
            "name": "frame 1 pane 160x48 coloured unchanged",
            "value": 2.899,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse",
            "value": 15.099,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse one row",
            "value": 3.032,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse unchanged",
            "value": 2.74,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured",
            "value": 159.318,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history",
            "value": 210.812,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history reflowed unchanged",
            "value": 7.904,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history unchanged",
            "value": 10.419,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured live",
            "value": 143.986,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row",
            "value": 7.281,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row live",
            "value": 140.636,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled",
            "value": 45.829,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled live",
            "value": 160.443,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured unchanged",
            "value": 5.833,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style",
            "value": 207.178,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style live",
            "value": 195.676,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style unchanged",
            "value": 5.801,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain",
            "value": 148.959,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history",
            "value": 150.83,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history reflowed unchanged",
            "value": 7.334,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history unchanged",
            "value": 10.524,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain live",
            "value": 120.136,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain unchanged",
            "value": 5.686,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode",
            "value": 149.375,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode history",
            "value": 351.53,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode live",
            "value": 127.587,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo",
            "value": 220.09,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo unchanged",
            "value": 5.894,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode unchanged",
            "value": 5.595,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured",
            "value": 48.324,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured unchanged",
            "value": 6.497,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain",
            "value": 10.145,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain live",
            "value": 9.362,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row",
            "value": 1.93,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row live",
            "value": 8.911,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled",
            "value": 3.522,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled live",
            "value": 8.921,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain unchanged",
            "value": 1.603,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo",
            "value": 15.311,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo unchanged",
            "value": 1.525,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images 10 placements",
            "value": 2.048,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images none",
            "value": 1.57,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images placeholders",
            "value": 2.144,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains",
            "value": 57.406,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains cold",
            "value": 2519.41,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains one row",
            "value": 3.361,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains rebuilt",
            "value": 29.64,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains scrolled",
            "value": 7.096,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains unchanged",
            "value": 1.711,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo",
            "value": 17.324,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo cold",
            "value": 17.197,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo one row",
            "value": 2.251,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo rebuilt",
            "value": 17.079,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo scrolled",
            "value": 5.483,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo unchanged",
            "value": 1.708,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains",
            "value": 578.25,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains cold",
            "value": 4076.005,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains one row",
            "value": 10.874,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains rebuilt",
            "value": 280.003,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains scrolled",
            "value": 42.603,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains unchanged",
            "value": 5.648,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo",
            "value": 161.452,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo cold",
            "value": 161.706,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo one row",
            "value": 6.774,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo rebuilt",
            "value": 159.29,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo scrolled",
            "value": 36.949,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo unchanged",
            "value": 5.343,
            "unit": "µs"
          },
          {
            "name": "history decode/background",
            "value": 1214.511,
            "unit": "µs"
          },
          {
            "name": "history decode/cjk",
            "value": 1784.304,
            "unit": "µs"
          },
          {
            "name": "history decode/emoji",
            "value": 2547.399,
            "unit": "µs"
          },
          {
            "name": "history decode/gradient",
            "value": 21887.349,
            "unit": "µs"
          },
          {
            "name": "history decode/ls",
            "value": 1726.836,
            "unit": "µs"
          },
          {
            "name": "history decode/plain",
            "value": 984.538,
            "unit": "µs"
          },
          {
            "name": "history push/background",
            "value": 2076.291,
            "unit": "µs"
          },
          {
            "name": "history push/cjk",
            "value": 1519.882,
            "unit": "µs"
          },
          {
            "name": "history push/emoji",
            "value": 1193.833,
            "unit": "µs"
          },
          {
            "name": "history push/gradient",
            "value": 29295.117,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/background",
            "value": 1848.763,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 1611.78,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 1120.613,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 28723.849,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/ls",
            "value": 846.147,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/plain",
            "value": 426.749,
            "unit": "µs"
          },
          {
            "name": "history push/ls",
            "value": 1007.015,
            "unit": "µs"
          },
          {
            "name": "history push/plain",
            "value": 411.142,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_200",
            "value": 20.876,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_300",
            "value": 10.813,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_200",
            "value": 117.745,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_300",
            "value": 36.119,
            "unit": "µs"
          },
          {
            "name": "parse/clusters",
            "value": 47110.349,
            "unit": "µs"
          },
          {
            "name": "parse/coloured",
            "value": 19864.961,
            "unit": "µs"
          },
          {
            "name": "parse/plain",
            "value": 8268.521,
            "unit": "µs"
          },
          {
            "name": "parse/seq",
            "value": 30615.76,
            "unit": "µs"
          },
          {
            "name": "parse/tui",
            "value": 12199.65,
            "unit": "µs"
          },
          {
            "name": "parse/unicode",
            "value": 31347.907,
            "unit": "µs"
          },
          {
            "name": "parse_1k/clusters",
            "value": 45260.317,
            "unit": "µs"
          },
          {
            "name": "parse_1k/coloured",
            "value": 19765.023,
            "unit": "µs"
          },
          {
            "name": "parse_1k/plain",
            "value": 7207.114,
            "unit": "µs"
          },
          {
            "name": "parse_1k/seq",
            "value": 29894.244,
            "unit": "µs"
          },
          {
            "name": "parse_1k/tui",
            "value": 11927.559,
            "unit": "µs"
          },
          {
            "name": "parse_1k/unicode",
            "value": 31126.215,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode",
            "value": 40.28,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode one row",
            "value": 41.273,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare",
            "value": 59.498,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare unchanged",
            "value": 3.024,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build",
            "value": 62.48,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build unchanged",
            "value": 6.328,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 encode",
            "value": 51.286,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_ascii",
            "value": 321.23,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_cjk",
            "value": 257.759,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_ascii",
            "value": 316.476,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_cjk",
            "value": 249.678,
            "unit": "µs"
          },
          {
            "name": "search_100k/filters/build_all",
            "value": 10814.387,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent",
            "value": 47.877,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent_ignore_case",
            "value": 55.135,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/common_ignore_case",
            "value": 5959.891,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/non_ascii_ignore_case",
            "value": 121.332,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare",
            "value": 55.189,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare_ignore_case",
            "value": 62.305,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent",
            "value": 391.649,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent_ignore_case",
            "value": 614.436,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/common_ignore_case",
            "value": 5964.676,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/non_ascii_ignore_case",
            "value": 605.585,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare",
            "value": 544.33,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare_ignore_case",
            "value": 871.856,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent",
            "value": 28.802,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent_ignore_case",
            "value": 27.548,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/common_ignore_case",
            "value": 30.476,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/non_ascii_ignore_case",
            "value": 28.269,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare",
            "value": 27.975,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare_ignore_case",
            "value": 27.587,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 full copy",
            "value": 11.83,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 history full copy",
            "value": 60.093,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 spinner",
            "value": 5.034,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 unchanged",
            "value": 4.594,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 full copy",
            "value": 1.042,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 spinner",
            "value": 1.485,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 unchanged",
            "value": 1.17,
            "unit": "µs"
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
        "date": 1793507908641,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "frame 1 pane 160x48 coloured",
            "value": 40.035,
            "unit": "µs"
          },
          {
            "name": "frame 1 pane 160x48 coloured unchanged",
            "value": 3.087,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse",
            "value": 13.684,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse one row",
            "value": 2.8,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse unchanged",
            "value": 2.496,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured",
            "value": 155.183,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history",
            "value": 198.182,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history reflowed unchanged",
            "value": 6.726,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history unchanged",
            "value": 9.053,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured live",
            "value": 122.464,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row",
            "value": 7.161,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row live",
            "value": 126.175,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled",
            "value": 42.152,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled live",
            "value": 126.12,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured unchanged",
            "value": 5.332,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style",
            "value": 185.687,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style live",
            "value": 166.769,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style unchanged",
            "value": 5.171,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain",
            "value": 134.459,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history",
            "value": 132.942,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history reflowed unchanged",
            "value": 6.444,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history unchanged",
            "value": 9.183,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain live",
            "value": 109.895,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain unchanged",
            "value": 5.231,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 prompts history unchanged",
            "value": 13.321,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode",
            "value": 131.599,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode history",
            "value": 314.005,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode live",
            "value": 110.781,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo",
            "value": 207.031,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo unchanged",
            "value": 4.995,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode unchanged",
            "value": 5.563,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured",
            "value": 45.45,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured unchanged",
            "value": 5.978,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain",
            "value": 9.77,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain live",
            "value": 7.61,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row",
            "value": 1.88,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row live",
            "value": 7.547,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled",
            "value": 3.461,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled live",
            "value": 7.409,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain unchanged",
            "value": 1.394,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo",
            "value": 14.984,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo unchanged",
            "value": 1.375,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images 10 placements",
            "value": 2.134,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images none",
            "value": 1.646,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images placeholders",
            "value": 2.161,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains",
            "value": 53.747,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains cold",
            "value": 2252.176,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains one row",
            "value": 3.304,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains rebuilt",
            "value": 26.078,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains scrolled",
            "value": 6.998,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains unchanged",
            "value": 1.705,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo",
            "value": 16.807,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo cold",
            "value": 16.999,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo one row",
            "value": 2.118,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo rebuilt",
            "value": 16.952,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo scrolled",
            "value": 5.113,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo unchanged",
            "value": 1.701,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains",
            "value": 601.23,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains cold",
            "value": 3987.373,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains one row",
            "value": 11.514,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains rebuilt",
            "value": 273.199,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains scrolled",
            "value": 41.747,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains unchanged",
            "value": 5.445,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo",
            "value": 149.864,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo cold",
            "value": 154.212,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo one row",
            "value": 6.726,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo rebuilt",
            "value": 159.054,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo scrolled",
            "value": 34.942,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo unchanged",
            "value": 4.858,
            "unit": "µs"
          },
          {
            "name": "history decode/background",
            "value": 811.421,
            "unit": "µs"
          },
          {
            "name": "history decode/cjk",
            "value": 1444.635,
            "unit": "µs"
          },
          {
            "name": "history decode/emoji",
            "value": 2042.166,
            "unit": "µs"
          },
          {
            "name": "history decode/gradient",
            "value": 16886.113,
            "unit": "µs"
          },
          {
            "name": "history decode/ls",
            "value": 1270.076,
            "unit": "µs"
          },
          {
            "name": "history decode/plain",
            "value": 683.968,
            "unit": "µs"
          },
          {
            "name": "history push/background",
            "value": 1474.017,
            "unit": "µs"
          },
          {
            "name": "history push/cjk",
            "value": 1138.597,
            "unit": "µs"
          },
          {
            "name": "history push/emoji",
            "value": 995.7,
            "unit": "µs"
          },
          {
            "name": "history push/gradient",
            "value": 23009.667,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/background",
            "value": 1478.974,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 1182.373,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 1088.299,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 24223.815,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/ls",
            "value": 746.594,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/plain",
            "value": 339.89,
            "unit": "µs"
          },
          {
            "name": "history push/ls",
            "value": 924.44,
            "unit": "µs"
          },
          {
            "name": "history push/plain",
            "value": 338.931,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_200",
            "value": 25.143,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_300",
            "value": 10.102,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_200",
            "value": 126.84,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_300",
            "value": 35.866,
            "unit": "µs"
          },
          {
            "name": "parse/clusters",
            "value": 42301.65,
            "unit": "µs"
          },
          {
            "name": "parse/coloured",
            "value": 16706.878,
            "unit": "µs"
          },
          {
            "name": "parse/plain",
            "value": 6928.599,
            "unit": "µs"
          },
          {
            "name": "parse/seq",
            "value": 28207.615,
            "unit": "µs"
          },
          {
            "name": "parse/tui",
            "value": 10273.006,
            "unit": "µs"
          },
          {
            "name": "parse/unicode",
            "value": 26004.555,
            "unit": "µs"
          },
          {
            "name": "parse_1k/clusters",
            "value": 49420.484,
            "unit": "µs"
          },
          {
            "name": "parse_1k/coloured",
            "value": 16412.055,
            "unit": "µs"
          },
          {
            "name": "parse_1k/plain",
            "value": 7053.031,
            "unit": "µs"
          },
          {
            "name": "parse_1k/seq",
            "value": 26680.602,
            "unit": "µs"
          },
          {
            "name": "parse_1k/tui",
            "value": 10679.272,
            "unit": "µs"
          },
          {
            "name": "parse_1k/unicode",
            "value": 27470.73,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode",
            "value": 11.228,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode one row",
            "value": 16.545,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare",
            "value": 60.017,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare unchanged",
            "value": 2.865,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build",
            "value": 62.581,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build unchanged",
            "value": 6.345,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 encode",
            "value": 12.45,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_ascii",
            "value": 281.486,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_cjk",
            "value": 258.398,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_ascii",
            "value": 280.298,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_cjk",
            "value": 250.65,
            "unit": "µs"
          },
          {
            "name": "search_100k/filters/build_all",
            "value": 9943.048,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent",
            "value": 41.672,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent_ignore_case",
            "value": 43.637,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/common_ignore_case",
            "value": 5588.701,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/non_ascii_ignore_case",
            "value": 108.689,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare",
            "value": 53.401,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare_ignore_case",
            "value": 57.997,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent",
            "value": 323.861,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent_ignore_case",
            "value": 520.303,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/common_ignore_case",
            "value": 5476.688,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/non_ascii_ignore_case",
            "value": 560.915,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare",
            "value": 493.456,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare_ignore_case",
            "value": 721.597,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent",
            "value": 26.648,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent_ignore_case",
            "value": 25.096,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/common_ignore_case",
            "value": 27.788,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/non_ascii_ignore_case",
            "value": 25.085,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare",
            "value": 26.983,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare_ignore_case",
            "value": 27.718,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 full copy",
            "value": 7.994,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 history full copy",
            "value": 62.133,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 spinner",
            "value": 4.562,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 unchanged",
            "value": 4.114,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 full copy",
            "value": 0.952,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 spinner",
            "value": 1.294,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 unchanged",
            "value": 1.052,
            "unit": "µs"
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
        "date": 1793594459529,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "frame 1 pane 160x48 coloured",
            "value": 46.202,
            "unit": "µs"
          },
          {
            "name": "frame 1 pane 160x48 coloured unchanged",
            "value": 3.015,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt coloured unchanged",
            "value": 2.929,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt list open unchanged",
            "value": 9.035,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt plain unchanged",
            "value": 2.7,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse",
            "value": 16.12,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse one row",
            "value": 3.265,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse unchanged",
            "value": 2.938,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured",
            "value": 148.605,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history",
            "value": 223.352,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history reflowed unchanged",
            "value": 7.552,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history smooth line",
            "value": 53.384,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history unchanged",
            "value": 10.348,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured live",
            "value": 131.861,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row",
            "value": 8.519,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row live",
            "value": 138.477,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled",
            "value": 41.513,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled live",
            "value": 140.343,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured unchanged",
            "value": 5.241,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style",
            "value": 234.088,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style live",
            "value": 207.331,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style unchanged",
            "value": 5.238,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain",
            "value": 124.622,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history",
            "value": 163.575,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history reflowed unchanged",
            "value": 7.146,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history smooth line",
            "value": 45.639,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history unchanged",
            "value": 10.333,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain live",
            "value": 109.714,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain unchanged",
            "value": 5.287,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 prompts history unchanged",
            "value": 15.049,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode",
            "value": 137.584,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode history",
            "value": 359.064,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode live",
            "value": 107.283,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo",
            "value": 214.756,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo unchanged",
            "value": 6.226,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode unchanged",
            "value": 5.145,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured",
            "value": 51.225,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured unchanged",
            "value": 6.686,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain",
            "value": 10.169,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain live",
            "value": 6.826,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row",
            "value": 2.043,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row live",
            "value": 7.728,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled",
            "value": 3.823,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled live",
            "value": 7.86,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain unchanged",
            "value": 1.558,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo",
            "value": 15.756,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo unchanged",
            "value": 1.425,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images 10 placements",
            "value": 2.245,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images none",
            "value": 1.599,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images placeholders",
            "value": 2.205,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains",
            "value": 50.996,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains cold",
            "value": 2226.635,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains one row",
            "value": 3.337,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains rebuilt",
            "value": 25.042,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains scrolled",
            "value": 6.416,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains unchanged",
            "value": 1.538,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo",
            "value": 18.049,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo cold",
            "value": 18.755,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo one row",
            "value": 2.055,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo rebuilt",
            "value": 17.023,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo scrolled",
            "value": 4.965,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo unchanged",
            "value": 1.527,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains",
            "value": 499.246,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains cold",
            "value": 3577.221,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains one row",
            "value": 11.057,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains rebuilt",
            "value": 259.772,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains scrolled",
            "value": 42.542,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains unchanged",
            "value": 5.972,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo",
            "value": 171.51,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo cold",
            "value": 172.396,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo one row",
            "value": 6.575,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo rebuilt",
            "value": 162.192,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo scrolled",
            "value": 32.05,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo unchanged",
            "value": 4.983,
            "unit": "µs"
          },
          {
            "name": "history decode/background",
            "value": 752.419,
            "unit": "µs"
          },
          {
            "name": "history decode/cjk",
            "value": 1513.643,
            "unit": "µs"
          },
          {
            "name": "history decode/emoji",
            "value": 1999.499,
            "unit": "µs"
          },
          {
            "name": "history decode/gradient",
            "value": 17360.38,
            "unit": "µs"
          },
          {
            "name": "history decode/ls",
            "value": 1286.59,
            "unit": "µs"
          },
          {
            "name": "history decode/plain",
            "value": 713.505,
            "unit": "µs"
          },
          {
            "name": "history push/background",
            "value": 1504.957,
            "unit": "µs"
          },
          {
            "name": "history push/cjk",
            "value": 1303.025,
            "unit": "µs"
          },
          {
            "name": "history push/emoji",
            "value": 1007.714,
            "unit": "µs"
          },
          {
            "name": "history push/gradient",
            "value": 24518.543,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/background",
            "value": 1501.349,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 1183.542,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 1063.074,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 23816.541,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/ls",
            "value": 711.883,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/plain",
            "value": 313.199,
            "unit": "µs"
          },
          {
            "name": "history push/ls",
            "value": 897.703,
            "unit": "µs"
          },
          {
            "name": "history push/plain",
            "value": 310.526,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_200",
            "value": 18.422,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_300",
            "value": 9.567,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_200",
            "value": 108.878,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_300",
            "value": 41.332,
            "unit": "µs"
          },
          {
            "name": "parse/clusters",
            "value": 36644.631,
            "unit": "µs"
          },
          {
            "name": "parse/coloured",
            "value": 16124.269,
            "unit": "µs"
          },
          {
            "name": "parse/plain",
            "value": 5508.664,
            "unit": "µs"
          },
          {
            "name": "parse/seq",
            "value": 18585.159,
            "unit": "µs"
          },
          {
            "name": "parse/tui",
            "value": 10209.89,
            "unit": "µs"
          },
          {
            "name": "parse/unicode",
            "value": 28268.049,
            "unit": "µs"
          },
          {
            "name": "parse_1k/clusters",
            "value": 39964.607,
            "unit": "µs"
          },
          {
            "name": "parse_1k/coloured",
            "value": 15516.686,
            "unit": "µs"
          },
          {
            "name": "parse_1k/plain",
            "value": 5967.775,
            "unit": "µs"
          },
          {
            "name": "parse_1k/seq",
            "value": 20498.051,
            "unit": "µs"
          },
          {
            "name": "parse_1k/tui",
            "value": 10532.087,
            "unit": "µs"
          },
          {
            "name": "parse_1k/unicode",
            "value": 28233.118,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode",
            "value": 8.125,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode one row",
            "value": 10.144,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare",
            "value": 62.838,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare unchanged",
            "value": 2.819,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build",
            "value": 69.45,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build unchanged",
            "value": 6.735,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 encode",
            "value": 13.169,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_ascii",
            "value": 297.274,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_cjk",
            "value": 300.702,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_ascii",
            "value": 319.992,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_cjk",
            "value": 262.888,
            "unit": "µs"
          },
          {
            "name": "search_100k/filters/build_all",
            "value": 9379.083,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent",
            "value": 44.736,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent_ignore_case",
            "value": 47.709,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/common_ignore_case",
            "value": 6976.344,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/non_ascii_ignore_case",
            "value": 108.139,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare",
            "value": 58.189,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare_ignore_case",
            "value": 56.509,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent",
            "value": 318.435,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent_ignore_case",
            "value": 523.309,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/common_ignore_case",
            "value": 7150.124,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/non_ascii_ignore_case",
            "value": 515.603,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare",
            "value": 587.847,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare_ignore_case",
            "value": 728.938,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent",
            "value": 25.069,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent_ignore_case",
            "value": 25.592,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/common_ignore_case",
            "value": 27.236,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/non_ascii_ignore_case",
            "value": 25.081,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare",
            "value": 27.288,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare_ignore_case",
            "value": 28.44,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 full copy",
            "value": 8.215,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 history full copy",
            "value": 54.472,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 spinner",
            "value": 4.549,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 unchanged",
            "value": 4.327,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 full copy",
            "value": 0.978,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 spinner",
            "value": 1.387,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 unchanged",
            "value": 1.102,
            "unit": "µs"
          },
          {
            "name": "typing/echo",
            "value": 1.934,
            "unit": "µs"
          },
          {
            "name": "typing/echo+report",
            "value": 19.486,
            "unit": "µs"
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
        "date": 1793614389477,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "frame 1 pane 160x48 coloured",
            "value": 52.036,
            "unit": "µs"
          },
          {
            "name": "frame 1 pane 160x48 coloured unchanged",
            "value": 3.455,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt coloured unchanged",
            "value": 3.693,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt list open unchanged",
            "value": 10.645,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt plain unchanged",
            "value": 3.52,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse",
            "value": 18.74,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse one row",
            "value": 3.761,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse unchanged",
            "value": 3.239,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured",
            "value": 220.68,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history",
            "value": 237.733,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history reflowed unchanged",
            "value": 10.005,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history smooth line",
            "value": 71.003,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history unchanged",
            "value": 12.737,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured live",
            "value": 159.02,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row",
            "value": 9.222,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row live",
            "value": 175.862,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled",
            "value": 64.11,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled live",
            "value": 203.568,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured unchanged",
            "value": 9.151,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style",
            "value": 294.075,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style live",
            "value": 257.816,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style unchanged",
            "value": 7.02,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain",
            "value": 155.59,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history",
            "value": 163.322,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history reflowed unchanged",
            "value": 8.159,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history smooth line",
            "value": 75.082,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history unchanged",
            "value": 12.053,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain live",
            "value": 116.482,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain unchanged",
            "value": 6.598,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 prompts history unchanged",
            "value": 18.922,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode",
            "value": 177.05,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode history",
            "value": 456.258,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode live",
            "value": 145.856,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo",
            "value": 286.547,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo unchanged",
            "value": 6.655,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode unchanged",
            "value": 6.622,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured",
            "value": 57.166,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured unchanged",
            "value": 7.839,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain",
            "value": 11.341,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain live",
            "value": 8.113,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row",
            "value": 2.168,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row live",
            "value": 8.096,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled",
            "value": 4.323,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled live",
            "value": 8.249,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain unchanged",
            "value": 1.994,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo",
            "value": 20.039,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo unchanged",
            "value": 1.759,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images 10 placements",
            "value": 2.328,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images none",
            "value": 2.073,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images placeholders",
            "value": 2.523,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains",
            "value": 63.205,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains cold",
            "value": 3352.693,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains one row",
            "value": 5.247,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains rebuilt",
            "value": 32.335,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains scrolled",
            "value": 12.804,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains unchanged",
            "value": 2.32,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo",
            "value": 19.638,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo cold",
            "value": 19.421,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo one row",
            "value": 2.744,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo rebuilt",
            "value": 22.134,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo scrolled",
            "value": 7.685,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo unchanged",
            "value": 2.013,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains",
            "value": 741.783,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains cold",
            "value": 4842.111,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains one row",
            "value": 15.775,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains rebuilt",
            "value": 384.248,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains scrolled",
            "value": 51.623,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains unchanged",
            "value": 6.724,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo",
            "value": 277.838,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo cold",
            "value": 226.384,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo one row",
            "value": 7.962,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo rebuilt",
            "value": 187.165,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo scrolled",
            "value": 43.561,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo unchanged",
            "value": 6.195,
            "unit": "µs"
          },
          {
            "name": "history decode/background",
            "value": 2253.147,
            "unit": "µs"
          },
          {
            "name": "history decode/cjk",
            "value": 2003.193,
            "unit": "µs"
          },
          {
            "name": "history decode/emoji",
            "value": 2981.065,
            "unit": "µs"
          },
          {
            "name": "history decode/gradient",
            "value": 18101.67,
            "unit": "µs"
          },
          {
            "name": "history decode/ls",
            "value": 2345.051,
            "unit": "µs"
          },
          {
            "name": "history decode/plain",
            "value": 1426.31,
            "unit": "µs"
          },
          {
            "name": "history push/background",
            "value": 2233.642,
            "unit": "µs"
          },
          {
            "name": "history push/cjk",
            "value": 1620.635,
            "unit": "µs"
          },
          {
            "name": "history push/emoji",
            "value": 1351.597,
            "unit": "µs"
          },
          {
            "name": "history push/gradient",
            "value": 36381.881,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/background",
            "value": 2083.704,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 1410.524,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 1217.236,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 34474.353,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/ls",
            "value": 1247.545,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/plain",
            "value": 430.295,
            "unit": "µs"
          },
          {
            "name": "history push/ls",
            "value": 1247.17,
            "unit": "µs"
          },
          {
            "name": "history push/plain",
            "value": 465.305,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_200",
            "value": 22.898,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_300",
            "value": 12.448,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_200",
            "value": 153.763,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_300",
            "value": 64.999,
            "unit": "µs"
          },
          {
            "name": "parse/clusters",
            "value": 44719.333,
            "unit": "µs"
          },
          {
            "name": "parse/coloured",
            "value": 22562.999,
            "unit": "µs"
          },
          {
            "name": "parse/plain",
            "value": 6920.794,
            "unit": "µs"
          },
          {
            "name": "parse/seq",
            "value": 26954.151,
            "unit": "µs"
          },
          {
            "name": "parse/tui",
            "value": 14965.879,
            "unit": "µs"
          },
          {
            "name": "parse/unicode",
            "value": 32581.052,
            "unit": "µs"
          },
          {
            "name": "parse_1k/clusters",
            "value": 46435.971,
            "unit": "µs"
          },
          {
            "name": "parse_1k/coloured",
            "value": 23514.401,
            "unit": "µs"
          },
          {
            "name": "parse_1k/plain",
            "value": 7051.622,
            "unit": "µs"
          },
          {
            "name": "parse_1k/seq",
            "value": 26234.512,
            "unit": "µs"
          },
          {
            "name": "parse_1k/tui",
            "value": 16228.902,
            "unit": "µs"
          },
          {
            "name": "parse_1k/unicode",
            "value": 32153.921,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode",
            "value": 40.211,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode one row",
            "value": 45.974,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare",
            "value": 72.244,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare unchanged",
            "value": 3.524,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build",
            "value": 89.024,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build unchanged",
            "value": 7.51,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 encode",
            "value": 52.059,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_ascii",
            "value": 382.014,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_cjk",
            "value": 390.774,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_ascii",
            "value": 376.571,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_cjk",
            "value": 347.892,
            "unit": "µs"
          },
          {
            "name": "search_100k/filters/build_all",
            "value": 10930.919,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent",
            "value": 62.734,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent_ignore_case",
            "value": 60.123,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/common_ignore_case",
            "value": 8823.095,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/non_ascii_ignore_case",
            "value": 130.069,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare",
            "value": 62.61,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare_ignore_case",
            "value": 59.36,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent",
            "value": 446.601,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent_ignore_case",
            "value": 787.738,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/common_ignore_case",
            "value": 9646.175,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/non_ascii_ignore_case",
            "value": 799.622,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare",
            "value": 755.884,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare_ignore_case",
            "value": 783.79,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent",
            "value": 46.779,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent_ignore_case",
            "value": 32.098,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/common_ignore_case",
            "value": 36.977,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/non_ascii_ignore_case",
            "value": 32.355,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare",
            "value": 32.039,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare_ignore_case",
            "value": 32.401,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 full copy",
            "value": 11.135,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 history full copy",
            "value": 80.772,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 spinner",
            "value": 7.48,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 unchanged",
            "value": 6.284,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 full copy",
            "value": 1.237,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 spinner",
            "value": 1.858,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 unchanged",
            "value": 1.419,
            "unit": "µs"
          },
          {
            "name": "typing/echo",
            "value": 2.268,
            "unit": "µs"
          },
          {
            "name": "typing/echo+report",
            "value": 19.885,
            "unit": "µs"
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
        "date": 1793680965039,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "frame 1 pane 160x48 coloured",
            "value": 48.481,
            "unit": "µs"
          },
          {
            "name": "frame 1 pane 160x48 coloured unchanged",
            "value": 3.477,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt coloured unchanged",
            "value": 3.722,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt list open unchanged",
            "value": 10.797,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt plain unchanged",
            "value": 3.521,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse",
            "value": 18.586,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse one row",
            "value": 3.625,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse unchanged",
            "value": 3.354,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured",
            "value": 187.215,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history",
            "value": 260.835,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history reflowed unchanged",
            "value": 9.242,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history smooth line",
            "value": 61.925,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history unchanged",
            "value": 12.491,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured live",
            "value": 162.238,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row",
            "value": 8.979,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row live",
            "value": 161.395,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled",
            "value": 56.745,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled live",
            "value": 177.275,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured unchanged",
            "value": 7.348,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style",
            "value": 307.461,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style live",
            "value": 274.922,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style unchanged",
            "value": 6.907,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain",
            "value": 174.604,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history",
            "value": 203.329,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history reflowed unchanged",
            "value": 8.709,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history smooth line",
            "value": 58.576,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history unchanged",
            "value": 13.19,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain live",
            "value": 118.96,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain unchanged",
            "value": 8.295,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 prompts history unchanged",
            "value": 19.607,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode",
            "value": 177.019,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode history",
            "value": 491.217,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode live",
            "value": 140.124,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo",
            "value": 360.271,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo unchanged",
            "value": 9.688,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode unchanged",
            "value": 7.328,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured",
            "value": 54.926,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured unchanged",
            "value": 7.521,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain",
            "value": 19.5,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain live",
            "value": 9.698,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row",
            "value": 3.036,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row live",
            "value": 11.149,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled",
            "value": 4.65,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled live",
            "value": 8.997,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain unchanged",
            "value": 2.99,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo",
            "value": 20.499,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo unchanged",
            "value": 2.219,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images 10 placements",
            "value": 2.505,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images none",
            "value": 2.023,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images placeholders",
            "value": 2.554,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains",
            "value": 67.561,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains cold",
            "value": 3389.073,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains one row",
            "value": 4.193,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains rebuilt",
            "value": 48.457,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains scrolled",
            "value": 9.346,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains unchanged",
            "value": 2.652,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo",
            "value": 22.471,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo cold",
            "value": 20.31,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo one row",
            "value": 2.78,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo rebuilt",
            "value": 21.358,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo scrolled",
            "value": 6.699,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo unchanged",
            "value": 2.21,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains",
            "value": 692.838,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains cold",
            "value": 4471.422,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains one row",
            "value": 13.296,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains rebuilt",
            "value": 353.094,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains scrolled",
            "value": 50.043,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains unchanged",
            "value": 7.181,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo",
            "value": 203.383,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo cold",
            "value": 190.251,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo one row",
            "value": 8.872,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo rebuilt",
            "value": 242.35,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo scrolled",
            "value": 47.325,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo unchanged",
            "value": 7.051,
            "unit": "µs"
          },
          {
            "name": "history decode/background",
            "value": 1415.226,
            "unit": "µs"
          },
          {
            "name": "history decode/cjk",
            "value": 2666.825,
            "unit": "µs"
          },
          {
            "name": "history decode/emoji",
            "value": 3705.99,
            "unit": "µs"
          },
          {
            "name": "history decode/gradient",
            "value": 21069.393,
            "unit": "µs"
          },
          {
            "name": "history decode/ls",
            "value": 1978.661,
            "unit": "µs"
          },
          {
            "name": "history decode/plain",
            "value": 976.149,
            "unit": "µs"
          },
          {
            "name": "history push/background",
            "value": 2070.809,
            "unit": "µs"
          },
          {
            "name": "history push/cjk",
            "value": 1525.154,
            "unit": "µs"
          },
          {
            "name": "history push/emoji",
            "value": 1127.489,
            "unit": "µs"
          },
          {
            "name": "history push/gradient",
            "value": 37690.079,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/background",
            "value": 1847.161,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 1654.033,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 1187.288,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 27749.738,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/ls",
            "value": 1070.009,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/plain",
            "value": 517.436,
            "unit": "µs"
          },
          {
            "name": "history push/ls",
            "value": 1225.75,
            "unit": "µs"
          },
          {
            "name": "history push/plain",
            "value": 527.281,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_200",
            "value": 27.552,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_300",
            "value": 14.664,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_200",
            "value": 154.793,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_300",
            "value": 59.986,
            "unit": "µs"
          },
          {
            "name": "parse/clusters",
            "value": 72482.94,
            "unit": "µs"
          },
          {
            "name": "parse/coloured",
            "value": 19475.609,
            "unit": "µs"
          },
          {
            "name": "parse/plain",
            "value": 7144.785,
            "unit": "µs"
          },
          {
            "name": "parse/seq",
            "value": 25470.749,
            "unit": "µs"
          },
          {
            "name": "parse/tui",
            "value": 12748.1,
            "unit": "µs"
          },
          {
            "name": "parse/unicode",
            "value": 35940.693,
            "unit": "µs"
          },
          {
            "name": "parse_1k/clusters",
            "value": 49122.788,
            "unit": "µs"
          },
          {
            "name": "parse_1k/coloured",
            "value": 20561.971,
            "unit": "µs"
          },
          {
            "name": "parse_1k/plain",
            "value": 9731.777,
            "unit": "µs"
          },
          {
            "name": "parse_1k/seq",
            "value": 24618.249,
            "unit": "µs"
          },
          {
            "name": "parse_1k/tui",
            "value": 13026.672,
            "unit": "µs"
          },
          {
            "name": "parse_1k/unicode",
            "value": 33773.209,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode",
            "value": 56.11,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode one row",
            "value": 49.038,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare",
            "value": 67.684,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare unchanged",
            "value": 3.41,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build",
            "value": 71.092,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build unchanged",
            "value": 8.215,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 encode",
            "value": 50.652,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_ascii",
            "value": 373.417,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_cjk",
            "value": 319.459,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_ascii",
            "value": 397.562,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_cjk",
            "value": 306.784,
            "unit": "µs"
          },
          {
            "name": "search_100k/filters/build_all",
            "value": 13442.092,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent",
            "value": 106.325,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent_ignore_case",
            "value": 71.424,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/common_ignore_case",
            "value": 10068.402,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/non_ascii_ignore_case",
            "value": 179.312,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare",
            "value": 73.303,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare_ignore_case",
            "value": 70.424,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent",
            "value": 488.051,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent_ignore_case",
            "value": 907.484,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/common_ignore_case",
            "value": 11839.758,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/non_ascii_ignore_case",
            "value": 1032.367,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare",
            "value": 643.511,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare_ignore_case",
            "value": 1006.113,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent",
            "value": 61.054,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent_ignore_case",
            "value": 37.209,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/common_ignore_case",
            "value": 42.685,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/non_ascii_ignore_case",
            "value": 39.884,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare",
            "value": 33.797,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare_ignore_case",
            "value": 38.468,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 full copy",
            "value": 13.33,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 history full copy",
            "value": 86.672,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 spinner",
            "value": 6.751,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 unchanged",
            "value": 5.485,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 full copy",
            "value": 1.385,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 spinner",
            "value": 1.979,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 unchanged",
            "value": 1.535,
            "unit": "µs"
          },
          {
            "name": "typing/echo",
            "value": 2.213,
            "unit": "µs"
          },
          {
            "name": "typing/echo+report",
            "value": 21.841,
            "unit": "µs"
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
        "date": 1793853871397,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "frame 1 pane 160x48 coloured",
            "value": 48.261,
            "unit": "µs"
          },
          {
            "name": "frame 1 pane 160x48 coloured unchanged",
            "value": 3.8,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt coloured unchanged",
            "value": 3.524,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt list open unchanged",
            "value": 10.034,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt plain unchanged",
            "value": 3.288,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse",
            "value": 17.698,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse one row",
            "value": 3.55,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse unchanged",
            "value": 3.217,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured",
            "value": 212.491,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history",
            "value": 320.011,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history reflowed unchanged",
            "value": 9.435,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history smooth line",
            "value": 61.792,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history unchanged",
            "value": 13.156,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured live",
            "value": 160.243,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row",
            "value": 9.866,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row live",
            "value": 151.39,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled",
            "value": 56.635,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled live",
            "value": 185.385,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured unchanged",
            "value": 9.11,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style",
            "value": 202.593,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style live",
            "value": 307.127,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style unchanged",
            "value": 7.221,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain",
            "value": 193.491,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history",
            "value": 171.197,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history reflowed unchanged",
            "value": 9.743,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history smooth line",
            "value": 58.672,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history unchanged",
            "value": 15.248,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain live",
            "value": 157.307,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain unchanged",
            "value": 10.846,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 prompts history unchanged",
            "value": 23.551,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode",
            "value": 177.619,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode history",
            "value": 624.565,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode live",
            "value": 160.807,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo",
            "value": 299.337,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo unchanged",
            "value": 6.944,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode unchanged",
            "value": 7.319,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured",
            "value": 61.76,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured unchanged",
            "value": 7.989,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain",
            "value": 12.71,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain live",
            "value": 9.554,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row",
            "value": 2.684,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row live",
            "value": 8.663,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled",
            "value": 4.605,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled live",
            "value": 9.405,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain unchanged",
            "value": 2.073,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo",
            "value": 22.222,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo unchanged",
            "value": 1.926,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images 10 placements",
            "value": 2.369,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images none",
            "value": 1.925,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images placeholders",
            "value": 2.474,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains",
            "value": 70.493,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains cold",
            "value": 3093.583,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains one row",
            "value": 4.383,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains rebuilt",
            "value": 33.247,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains scrolled",
            "value": 9.426,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains unchanged",
            "value": 2.185,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo",
            "value": 18.936,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo cold",
            "value": 18.901,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo one row",
            "value": 2.655,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo rebuilt",
            "value": 18.216,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo scrolled",
            "value": 6.233,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo unchanged",
            "value": 2.088,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains",
            "value": 659.465,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains cold",
            "value": 4744.842,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains one row",
            "value": 12.125,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains rebuilt",
            "value": 335.846,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains scrolled",
            "value": 46.735,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains unchanged",
            "value": 6.134,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo",
            "value": 185.824,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo cold",
            "value": 178.142,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo one row",
            "value": 9.002,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo rebuilt",
            "value": 182.183,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo scrolled",
            "value": 49.35,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo unchanged",
            "value": 6.903,
            "unit": "µs"
          },
          {
            "name": "history decode/background",
            "value": 1274.474,
            "unit": "µs"
          },
          {
            "name": "history decode/cjk",
            "value": 2212.057,
            "unit": "µs"
          },
          {
            "name": "history decode/emoji",
            "value": 2773.269,
            "unit": "µs"
          },
          {
            "name": "history decode/gradient",
            "value": 20759.422,
            "unit": "µs"
          },
          {
            "name": "history decode/ls",
            "value": 1870.173,
            "unit": "µs"
          },
          {
            "name": "history decode/plain",
            "value": 1194.136,
            "unit": "µs"
          },
          {
            "name": "history push/background",
            "value": 2130.635,
            "unit": "µs"
          },
          {
            "name": "history push/cjk",
            "value": 1384.546,
            "unit": "µs"
          },
          {
            "name": "history push/emoji",
            "value": 1553.793,
            "unit": "µs"
          },
          {
            "name": "history push/gradient",
            "value": 30803.097,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/background",
            "value": 1856.24,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 1528.912,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 1336.295,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 29965.381,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/ls",
            "value": 845.597,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/plain",
            "value": 414.835,
            "unit": "µs"
          },
          {
            "name": "history push/ls",
            "value": 1360.724,
            "unit": "µs"
          },
          {
            "name": "history push/plain",
            "value": 445.096,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_200",
            "value": 32.164,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_300",
            "value": 13.787,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_200",
            "value": 175.904,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_300",
            "value": 57.281,
            "unit": "µs"
          },
          {
            "name": "parse/clusters",
            "value": 48308.538,
            "unit": "µs"
          },
          {
            "name": "parse/coloured",
            "value": 21118.303,
            "unit": "µs"
          },
          {
            "name": "parse/plain",
            "value": 9125.039,
            "unit": "µs"
          },
          {
            "name": "parse/seq",
            "value": 26116.876,
            "unit": "µs"
          },
          {
            "name": "parse/tui",
            "value": 13302.069,
            "unit": "µs"
          },
          {
            "name": "parse/unicode",
            "value": 33651.941,
            "unit": "µs"
          },
          {
            "name": "parse_1k/clusters",
            "value": 48016.661,
            "unit": "µs"
          },
          {
            "name": "parse_1k/coloured",
            "value": 21442.75,
            "unit": "µs"
          },
          {
            "name": "parse_1k/plain",
            "value": 8912.964,
            "unit": "µs"
          },
          {
            "name": "parse_1k/seq",
            "value": 25576.608,
            "unit": "µs"
          },
          {
            "name": "parse_1k/tui",
            "value": 13444.264,
            "unit": "µs"
          },
          {
            "name": "parse_1k/unicode",
            "value": 34584.129,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode",
            "value": 40.002,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode one row",
            "value": 45.205,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare",
            "value": 54.277,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare unchanged",
            "value": 3.317,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build",
            "value": 72.024,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build unchanged",
            "value": 7.361,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 encode",
            "value": 51.487,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_ascii",
            "value": 395.064,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_cjk",
            "value": 323.014,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_ascii",
            "value": 385.857,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_cjk",
            "value": 311.17,
            "unit": "µs"
          },
          {
            "name": "search_100k/filters/build_all",
            "value": 16769.755,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent",
            "value": 71.393,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent_ignore_case",
            "value": 64.487,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/common_ignore_case",
            "value": 9080.079,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/non_ascii_ignore_case",
            "value": 138.938,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare",
            "value": 69.535,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare_ignore_case",
            "value": 70.575,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent",
            "value": 531.087,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent_ignore_case",
            "value": 739.683,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/common_ignore_case",
            "value": 9100.518,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/non_ascii_ignore_case",
            "value": 721.368,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare",
            "value": 723.483,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare_ignore_case",
            "value": 1132.049,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent",
            "value": 35.954,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent_ignore_case",
            "value": 37.476,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/common_ignore_case",
            "value": 37.416,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/non_ascii_ignore_case",
            "value": 32.985,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare",
            "value": 36.57,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare_ignore_case",
            "value": 32.317,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 full copy",
            "value": 10.784,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 history full copy",
            "value": 69.885,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 spinner",
            "value": 5.778,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 unchanged",
            "value": 5.428,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 full copy",
            "value": 1.386,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 spinner",
            "value": 2.045,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 unchanged",
            "value": 1.381,
            "unit": "µs"
          },
          {
            "name": "typing/echo",
            "value": 2.601,
            "unit": "µs"
          },
          {
            "name": "typing/echo+report",
            "value": 23.007,
            "unit": "µs"
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
          "id": "a4086db1155e9f82785501269214d3fdbadf2f73",
          "message": "docs(update): design What's New (updater.md §12)",
          "timestamp": "2026-11-05T12:13:04Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/a4086db1155e9f82785501269214d3fdbadf2f73"
        },
        "date": 1793940148272,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "frame 1 pane 160x48 coloured",
            "value": 48.338,
            "unit": "µs"
          },
          {
            "name": "frame 1 pane 160x48 coloured unchanged",
            "value": 3.122,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt coloured unchanged",
            "value": 3.645,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt list open unchanged",
            "value": 11.061,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt plain unchanged",
            "value": 3.173,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse",
            "value": 15.349,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse one row",
            "value": 3.422,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse unchanged",
            "value": 3.242,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured",
            "value": 163.735,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history",
            "value": 213.324,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history reflowed unchanged",
            "value": 8.071,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history smooth line",
            "value": 55.151,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history unchanged",
            "value": 11.575,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured live",
            "value": 142.102,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row",
            "value": 8.577,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row live",
            "value": 134.907,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled",
            "value": 49.095,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled live",
            "value": 145.247,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured unchanged",
            "value": 6.536,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style",
            "value": 187.243,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style live",
            "value": 247.782,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style unchanged",
            "value": 7.269,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain",
            "value": 142.122,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history",
            "value": 145.92,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history reflowed unchanged",
            "value": 8.198,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history smooth line",
            "value": 50.043,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history unchanged",
            "value": 11.502,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain live",
            "value": 109.665,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain unchanged",
            "value": 6.239,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 prompts history unchanged",
            "value": 15.943,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode",
            "value": 160.185,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode history",
            "value": 348.05,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode live",
            "value": 117.053,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo",
            "value": 261.757,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo unchanged",
            "value": 6.084,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode unchanged",
            "value": 6.391,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured",
            "value": 49.025,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured unchanged",
            "value": 7.486,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain",
            "value": 10.745,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain live",
            "value": 7.889,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row",
            "value": 2.064,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row live",
            "value": 7.884,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled",
            "value": 3.668,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled live",
            "value": 7.605,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain unchanged",
            "value": 1.757,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo",
            "value": 17.97,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo unchanged",
            "value": 1.657,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images 10 placements",
            "value": 2.192,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images none",
            "value": 1.85,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images placeholders",
            "value": 2.384,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains",
            "value": 59.008,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains cold",
            "value": 2453.313,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains one row",
            "value": 3.675,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains rebuilt",
            "value": 29.261,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains scrolled",
            "value": 7.667,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains unchanged",
            "value": 1.832,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo",
            "value": 17.834,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo cold",
            "value": 16.711,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo one row",
            "value": 2.511,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo rebuilt",
            "value": 17.272,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo scrolled",
            "value": 5.521,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo unchanged",
            "value": 1.973,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains",
            "value": 593.729,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains cold",
            "value": 4259.954,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains one row",
            "value": 11.675,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains rebuilt",
            "value": 282.106,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains scrolled",
            "value": 46.752,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains unchanged",
            "value": 6.005,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo",
            "value": 145.755,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo cold",
            "value": 147.625,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo one row",
            "value": 7.92,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo rebuilt",
            "value": 155.295,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo scrolled",
            "value": 38.514,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo unchanged",
            "value": 5.556,
            "unit": "µs"
          },
          {
            "name": "history decode/background",
            "value": 1123.73,
            "unit": "µs"
          },
          {
            "name": "history decode/cjk",
            "value": 1947.181,
            "unit": "µs"
          },
          {
            "name": "history decode/emoji",
            "value": 2565.966,
            "unit": "µs"
          },
          {
            "name": "history decode/gradient",
            "value": 19567.637,
            "unit": "µs"
          },
          {
            "name": "history decode/ls",
            "value": 1704.313,
            "unit": "µs"
          },
          {
            "name": "history decode/plain",
            "value": 1221.596,
            "unit": "µs"
          },
          {
            "name": "history push/background",
            "value": 2422.24,
            "unit": "µs"
          },
          {
            "name": "history push/cjk",
            "value": 1399.693,
            "unit": "µs"
          },
          {
            "name": "history push/emoji",
            "value": 1133.709,
            "unit": "µs"
          },
          {
            "name": "history push/gradient",
            "value": 34684.003,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/background",
            "value": 1656.343,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 1448.007,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 1160.256,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 29621.082,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/ls",
            "value": 850.519,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/plain",
            "value": 447.152,
            "unit": "µs"
          },
          {
            "name": "history push/ls",
            "value": 1029.048,
            "unit": "µs"
          },
          {
            "name": "history push/plain",
            "value": 611.269,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_200",
            "value": 33.491,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_300",
            "value": 10.914,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_200",
            "value": 127.953,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_300",
            "value": 46.419,
            "unit": "µs"
          },
          {
            "name": "parse/clusters",
            "value": 42951.295,
            "unit": "µs"
          },
          {
            "name": "parse/coloured",
            "value": 19362.378,
            "unit": "µs"
          },
          {
            "name": "parse/plain",
            "value": 6381.222,
            "unit": "µs"
          },
          {
            "name": "parse/seq",
            "value": 29838.369,
            "unit": "µs"
          },
          {
            "name": "parse/tui",
            "value": 11569.524,
            "unit": "µs"
          },
          {
            "name": "parse/unicode",
            "value": 32605.413,
            "unit": "µs"
          },
          {
            "name": "parse_1k/clusters",
            "value": 43501.457,
            "unit": "µs"
          },
          {
            "name": "parse_1k/coloured",
            "value": 18079.137,
            "unit": "µs"
          },
          {
            "name": "parse_1k/plain",
            "value": 6741.848,
            "unit": "µs"
          },
          {
            "name": "parse_1k/seq",
            "value": 21036.288,
            "unit": "µs"
          },
          {
            "name": "parse_1k/tui",
            "value": 11644.137,
            "unit": "µs"
          },
          {
            "name": "parse_1k/unicode",
            "value": 31604.037,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode",
            "value": 40.507,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode one row",
            "value": 47.914,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare",
            "value": 56.234,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare unchanged",
            "value": 3.111,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build",
            "value": 85.367,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build unchanged",
            "value": 8.271,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 encode",
            "value": 50.618,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_ascii",
            "value": 329.463,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_cjk",
            "value": 275.408,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_ascii",
            "value": 379.688,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_cjk",
            "value": 280.787,
            "unit": "µs"
          },
          {
            "name": "search_100k/filters/build_all",
            "value": 11079.212,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent",
            "value": 50.921,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent_ignore_case",
            "value": 60.925,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/common_ignore_case",
            "value": 8172.413,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/non_ascii_ignore_case",
            "value": 120.17,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare",
            "value": 58.455,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare_ignore_case",
            "value": 61.527,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent",
            "value": 399.657,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent_ignore_case",
            "value": 638.687,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/common_ignore_case",
            "value": 7529.199,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/non_ascii_ignore_case",
            "value": 640.099,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare",
            "value": 535.125,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare_ignore_case",
            "value": 887.289,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent",
            "value": 28.457,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent_ignore_case",
            "value": 30.603,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/common_ignore_case",
            "value": 32.452,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/non_ascii_ignore_case",
            "value": 29.48,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare",
            "value": 29.326,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare_ignore_case",
            "value": 29.445,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 full copy",
            "value": 9.156,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 history full copy",
            "value": 63.854,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 spinner",
            "value": 5.359,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 unchanged",
            "value": 4.793,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 full copy",
            "value": 1.159,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 spinner",
            "value": 1.626,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 unchanged",
            "value": 1.225,
            "unit": "µs"
          },
          {
            "name": "typing/echo",
            "value": 2.056,
            "unit": "µs"
          },
          {
            "name": "typing/echo+report",
            "value": 19.654,
            "unit": "µs"
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
          "id": "14f1c8e206d5d978b07ae61a8f513f50034438b4",
          "message": "fix: tighten terminal preview typography",
          "timestamp": "2026-11-07T10:52:42Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/14f1c8e206d5d978b07ae61a8f513f50034438b4"
        },
        "date": 1794112902570,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "frame 1 pane 160x48 coloured",
            "value": 46.192,
            "unit": "µs"
          },
          {
            "name": "frame 1 pane 160x48 coloured unchanged",
            "value": 3.381,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt coloured unchanged",
            "value": 3.741,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt list open unchanged",
            "value": 12.766,
            "unit": "µs"
          },
          {
            "name": "frame 160x48 prompt plain unchanged",
            "value": 3.466,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse",
            "value": 17.628,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse one row",
            "value": 3.394,
            "unit": "µs"
          },
          {
            "name": "frame 203x50 sparse unchanged",
            "value": 3.296,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured",
            "value": 246.335,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history",
            "value": 238.417,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history reflowed unchanged",
            "value": 9.016,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history smooth line",
            "value": 62.414,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured history unchanged",
            "value": 12.534,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured live",
            "value": 186.103,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row",
            "value": 8.724,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured one row live",
            "value": 145.073,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled",
            "value": 59.51,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured scrolled live",
            "value": 149.091,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 coloured unchanged",
            "value": 8.925,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style",
            "value": 259.513,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style live",
            "value": 338.267,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 per-cell style unchanged",
            "value": 8.681,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain",
            "value": 186.026,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history",
            "value": 229.854,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history reflowed unchanged",
            "value": 9.634,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history smooth line",
            "value": 64.323,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain history unchanged",
            "value": 12.901,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain live",
            "value": 202.431,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 plain unchanged",
            "value": 9.981,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 prompts history unchanged",
            "value": 17.339,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode",
            "value": 291.947,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode history",
            "value": 392.282,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode live",
            "value": 170.412,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo",
            "value": 330.16,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode menlo unchanged",
            "value": 7.286,
            "unit": "µs"
          },
          {
            "name": "frame 300x100 unicode unchanged",
            "value": 7.196,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured",
            "value": 55.244,
            "unit": "µs"
          },
          {
            "name": "frame 4 panes 80x24 coloured unchanged",
            "value": 7.284,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain",
            "value": 14.243,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain live",
            "value": 10.761,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row",
            "value": 2.199,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain one row live",
            "value": 7.908,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled",
            "value": 4.078,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain scrolled live",
            "value": 8.028,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 plain unchanged",
            "value": 2.676,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo",
            "value": 26.626,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 unicode menlo unchanged",
            "value": 2.509,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images 10 placements",
            "value": 2.564,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images none",
            "value": 2.163,
            "unit": "µs"
          },
          {
            "name": "frame 80x24 with_images placeholders",
            "value": 2.725,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains",
            "value": 64.379,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains cold",
            "value": 2652.42,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains one row",
            "value": 4.129,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains rebuilt",
            "value": 29.64,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains scrolled",
            "value": 8.878,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 jetbrains unchanged",
            "value": 2.117,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo",
            "value": 17.633,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo cold",
            "value": 18.004,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo one row",
            "value": 2.768,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo rebuilt",
            "value": 20.883,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo scrolled",
            "value": 6.253,
            "unit": "µs"
          },
          {
            "name": "frame code 100x30 menlo unchanged",
            "value": 1.945,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains",
            "value": 763.974,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains cold",
            "value": 5825.752,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains one row",
            "value": 13.912,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains rebuilt",
            "value": 305.686,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains scrolled",
            "value": 50.255,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 jetbrains unchanged",
            "value": 6.848,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo",
            "value": 156.436,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo cold",
            "value": 159.549,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo one row",
            "value": 7.718,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo rebuilt",
            "value": 161.232,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo scrolled",
            "value": 38.929,
            "unit": "µs"
          },
          {
            "name": "frame code 300x100 menlo unchanged",
            "value": 6.04,
            "unit": "µs"
          },
          {
            "name": "history decode/background",
            "value": 1467.024,
            "unit": "µs"
          },
          {
            "name": "history decode/cjk",
            "value": 2298.596,
            "unit": "µs"
          },
          {
            "name": "history decode/emoji",
            "value": 3238.39,
            "unit": "µs"
          },
          {
            "name": "history decode/gradient",
            "value": 21311.903,
            "unit": "µs"
          },
          {
            "name": "history decode/ls",
            "value": 1939.256,
            "unit": "µs"
          },
          {
            "name": "history decode/plain",
            "value": 1128.954,
            "unit": "µs"
          },
          {
            "name": "history push/background",
            "value": 2069.341,
            "unit": "µs"
          },
          {
            "name": "history push/cjk",
            "value": 1805.852,
            "unit": "µs"
          },
          {
            "name": "history push/emoji",
            "value": 1508.573,
            "unit": "µs"
          },
          {
            "name": "history push/gradient",
            "value": 36085.306,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/background",
            "value": 2071.446,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/cjk",
            "value": 1758.301,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/emoji",
            "value": 1513.61,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/gradient",
            "value": 33508.064,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/ls",
            "value": 1016.813,
            "unit": "µs"
          },
          {
            "name": "history push/keyed/plain",
            "value": 439.43,
            "unit": "µs"
          },
          {
            "name": "history push/ls",
            "value": 1163.67,
            "unit": "µs"
          },
          {
            "name": "history push/plain",
            "value": 424.113,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_200",
            "value": 34.995,
            "unit": "µs"
          },
          {
            "name": "history_at_width/display_offset_at_300",
            "value": 14.791,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_200",
            "value": 173.691,
            "unit": "µs"
          },
          {
            "name": "history_at_width/scroll_page_at_300",
            "value": 74.485,
            "unit": "µs"
          },
          {
            "name": "parse/clusters",
            "value": 47193.372,
            "unit": "µs"
          },
          {
            "name": "parse/coloured",
            "value": 29355.452,
            "unit": "µs"
          },
          {
            "name": "parse/plain",
            "value": 8897.184,
            "unit": "µs"
          },
          {
            "name": "parse/seq",
            "value": 28286.206,
            "unit": "µs"
          },
          {
            "name": "parse/tui",
            "value": 17621.411,
            "unit": "µs"
          },
          {
            "name": "parse/unicode",
            "value": 34994.582,
            "unit": "µs"
          },
          {
            "name": "parse_1k/clusters",
            "value": 47121.304,
            "unit": "µs"
          },
          {
            "name": "parse_1k/coloured",
            "value": 20153.549,
            "unit": "µs"
          },
          {
            "name": "parse_1k/plain",
            "value": 7312.165,
            "unit": "µs"
          },
          {
            "name": "parse_1k/seq",
            "value": 24909.739,
            "unit": "µs"
          },
          {
            "name": "parse_1k/tui",
            "value": 12438.777,
            "unit": "µs"
          },
          {
            "name": "parse_1k/unicode",
            "value": 32919.216,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode",
            "value": 48.167,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 encode one row",
            "value": 47.915,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare",
            "value": 60.516,
            "unit": "µs"
          },
          {
            "name": "renderer 1 pane 160x48 prepare unchanged",
            "value": 3.57,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build",
            "value": 67.237,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 build unchanged",
            "value": 8.055,
            "unit": "µs"
          },
          {
            "name": "renderer 4 panes 80x24 encode",
            "value": 57.728,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_ascii",
            "value": 370.681,
            "unit": "µs"
          },
          {
            "name": "resize/height_300x100_cjk",
            "value": 357.878,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_ascii",
            "value": 424.739,
            "unit": "µs"
          },
          {
            "name": "resize/width_300x100_cjk",
            "value": 403.508,
            "unit": "µs"
          },
          {
            "name": "search_100k/filters/build_all",
            "value": 13789.985,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent",
            "value": 55.418,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/absent_ignore_case",
            "value": 57.577,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/common_ignore_case",
            "value": 8927.277,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/non_ascii_ignore_case",
            "value": 129.816,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare",
            "value": 64.081,
            "unit": "µs"
          },
          {
            "name": "search_100k/job/rare_ignore_case",
            "value": 74.806,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent",
            "value": 431.435,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/absent_ignore_case",
            "value": 663.245,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/common_ignore_case",
            "value": 9595.568,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/non_ascii_ignore_case",
            "value": 695.842,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare",
            "value": 716.968,
            "unit": "µs"
          },
          {
            "name": "search_100k/scan/rare_ignore_case",
            "value": 1010.633,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent",
            "value": 35.534,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/absent_ignore_case",
            "value": 29.903,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/common_ignore_case",
            "value": 37.466,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/non_ascii_ignore_case",
            "value": 32.354,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare",
            "value": 34.924,
            "unit": "µs"
          },
          {
            "name": "search_100k/start/rare_ignore_case",
            "value": 35.524,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 full copy",
            "value": 9.919,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 history full copy",
            "value": 74.81,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 spinner",
            "value": 5.826,
            "unit": "µs"
          },
          {
            "name": "snapshot 300x100 unchanged",
            "value": 5.442,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 full copy",
            "value": 1.18,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 spinner",
            "value": 1.683,
            "unit": "µs"
          },
          {
            "name": "snapshot 80x24 unchanged",
            "value": 1.349,
            "unit": "µs"
          },
          {
            "name": "typing/echo",
            "value": 2.321,
            "unit": "µs"
          },
          {
            "name": "typing/echo+report",
            "value": 25.885,
            "unit": "µs"
          }
        ]
      }
    ]
  }
}