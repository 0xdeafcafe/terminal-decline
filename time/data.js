window.BENCHMARK_DATA = {
  "lastUpdate": 1793335307476,
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
      }
    ]
  }
}