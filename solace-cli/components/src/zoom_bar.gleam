//// A bar of zoom controls shared by pages with a drawing, and the tips that
//// name their shortcuts after a pause of hovering.

import gleam/float
import gleam/int
import lustre/attribute
import lustre/element.{type Element}
import lustre/element/html
import lustre/event

/// The tip each button shows after two seconds of hovering without a click.
pub type Tips {
  Tips(zoom_in: String, zoom_out: String, reset: String)
}

pub const styles =
  ".zoom-bar{display:flex;align-items:center;gap:2px;padding:2px;box-sizing:border-box;background:var(--paper,#fffefa);border:1px solid var(--line,#94a7a5)}
.zoom-bar button{position:relative;min-width:26px;height:26px;padding:0 6px;font:inherit;font-size:14px;color:var(--ink,#172226);background:transparent;border:1px solid transparent;cursor:pointer}
.zoom-bar button:hover{background:var(--hover,#e7eeea)}
.zoom-bar .zoom-level{min-width:48px;text-align:center;font-size:13px;font-variant-numeric:tabular-nums;color:var(--soft,#46575a)}
.zoom-bar button::after{content:attr(data-tip);position:absolute;right:0;bottom:calc(100% + 6px);padding:3px 7px;background:var(--ink,#172226);color:var(--paper,#fffefa);font:12px/1.35 'IBM Plex Sans',system-ui,sans-serif;white-space:nowrap;pointer-events:none;opacity:0;visibility:hidden;transition:opacity 0s linear 0s,visibility 0s linear 0s}
.zoom-bar button:hover::after{opacity:1;visibility:visible;transition-delay:2s}
.zoom-bar button:focus::after{opacity:0;visibility:hidden;transition-delay:0s}
.zoom-bar button:focus-visible{outline:2px solid var(--focus,#174f78);outline-offset:-2px}"

/// Zoom out, the current level, zoom in, and a reset to the default view.
pub fn bar(
  zoom: Float,
  zoom_in: msg,
  zoom_out: msg,
  reset: msg,
  tips: Tips,
) -> Element(msg) {
  html.div(
    [
      attribute.class("zoom-bar"),
      attribute.role("group"),
      attribute.aria_label("Zoom"),
    ],
    [
      button("Zoom out", "−", tips.zoom_out, zoom_out),
      html.span(
        [
          attribute.class("zoom-level"),
          attribute.aria_label("Zoom level " <> percent(zoom)),
        ],
        [html.text(percent(zoom))],
      ),
      button("Zoom in", "+", tips.zoom_in, zoom_in),
      button("Reset view", "Reset", tips.reset, reset),
    ],
  )
}

fn button(name: String, label: String, tip: String, msg: msg) -> Element(msg) {
  html.button(
    [
      attribute.type_("button"),
      attribute.aria_label(name),
      attribute.data("tip", tip),
      event.on_click(msg),
    ],
    [html.text(label)],
  )
}

fn percent(zoom: Float) -> String {
  int.to_string(float.round(zoom *. 100.0)) <> "%"
}
