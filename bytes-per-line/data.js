window.BENCHMARK_DATA = {
  "lastUpdate": 1793335299500,
  "repoUrl": "https://github.com/0xdeafcafe/terminal-decline",
  "entries": {
    "Scrollback bytes per line": [
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
        "date": 1793247584232,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 90,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 183,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 93,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 52,
            "unit": "bytes/line"
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
        "date": 1793335297690,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 69,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 67,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback emoji-heavy",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 30,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback gradient 160 cells",
            "value": 2019,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 61,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback placeholder image row",
            "value": 57,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 44,
            "unit": "bytes/line"
          }
        ]
      }
    ]
  }
}