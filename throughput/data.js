window.BENCHMARK_DATA = {
  "lastUpdate": 1793335303514,
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
      }
    ]
  }
}