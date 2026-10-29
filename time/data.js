window.BENCHMARK_DATA = {
  "lastUpdate": 1793247591361,
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
      }
    ]
  }
}