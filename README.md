# IFern

A personalized fork of [Warp](https://www.warp.dev) — the agentic development environment. Same engine underneath, custom branding and a few personal features layered on top.

## What's different from upstream

- **Embedded browser panel** — a real `WKWebView` in the left sidebar (globe icon, `Ctrl+5`). Type a URL or a search query and browse without leaving the terminal. Sign-in flows, popups, trackpad gestures, and pinch-zoom all work. (See [`browser_view.rs`](app/src/workspace/view/browser_view.rs) and PR [#1](https://github.com/Fernandoleano/IFern/pull/1).)
- **Chrome-style omnibox** — bare hostnames (`youtube.com`) get `https://`, anything without a dot falls back to a Google search.
- **Rebranded UI** — window title, menu bar, update banners all read **IFern** instead of Warp.

## Building

Same flow as upstream Warp:

```bash
./script/bootstrap   # platform-specific setup
./script/run         # build and run IFern
./script/presubmit   # fmt, clippy, and tests
```

See [WARP.md](WARP.md) for the full engineering guide.

## Status

Mac-first development. The embedded browser panel is macOS-only (uses `WKWebView`). Other platforms still build and run, they just don't get the browser panel.

## Credit

Built on top of [Warp](https://github.com/warpdotdev/warp) by Warp Inc., licensed under the [MIT](LICENSE-MIT) (`warpui_core` / `warpui`) and [AGPL v3](LICENSE-AGPL) (everything else).

This fork is for personal use and experimentation.
