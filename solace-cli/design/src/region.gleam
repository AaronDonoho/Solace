//// Works out which drawn elements lie wholly inside a dragged-out region.

import gleam/float
import gleam/list

/// A rectangle in viewport coordinates: left and top are the near corner,
/// right and bottom the far one.
pub type Rect {
  Rect(left: Float, top: Float, right: Float, bottom: Float)
}

/// The rectangle between two pointer positions, in either order.
pub fn between(
  start_x: Float,
  start_y: Float,
  end_x: Float,
  end_y: Float,
) -> Rect {
  Rect(
    left: float.min(start_x, end_x),
    top: float.min(start_y, end_y),
    right: float.max(start_x, end_x),
    bottom: float.max(start_y, end_y),
  )
}

/// The ids whose bounds lie wholly inside `area`, in the order given.
pub fn ids_inside(area: Rect, bounds: List(#(String, Rect))) -> List(String) {
  list.filter_map(bounds, fn(element) {
    let #(id, rect) = element
    case inside(area, rect) {
      True -> Ok(id)
      False -> Error(Nil)
    }
  })
}

fn inside(area: Rect, bounds: Rect) -> Bool {
  bounds.left >=. area.left
  && bounds.top >=. area.top
  && bounds.right <=. area.right
  && bounds.bottom <=. area.bottom
}
