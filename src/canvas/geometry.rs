#[derive(Debug, Clone, Copy)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Bounds {
    pub fn from_two(a: Pos, b: Pos) -> Self {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        let w = (a.x - b.x).abs();
        let h = (a.y - b.y).abs();
        Self { x, y, w, h }
    }

    pub fn centered(cx: f32, cy: f32, w: f32, h: f32) -> Self {
        Self {
            x: cx - w * 0.5,
            y: cy - h * 0.5,
            w,
            h,
        }
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn center(self) -> Pos {
        Pos {
            x: self.x + self.w * 0.5,
            y: self.y + self.h * 0.5,
        }
    }

    pub fn nw(self) -> Pos {
        Pos { x: self.x, y: self.y }
    }

    pub fn ne(self) -> Pos {
        Pos {
            x: self.right(),
            y: self.y,
        }
    }

    pub fn se(self) -> Pos {
        Pos {
            x: self.right(),
            y: self.bottom(),
        }
    }

    pub fn sw(self) -> Pos {
        Pos {
            x: self.x,
            y: self.bottom(),
        }
    }

    pub fn union(self, o: Self) -> Self {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let r = self.right().max(o.right());
        let b = self.bottom().max(o.bottom());
        Self {
            x,
            y,
            w: (r - x).max(0.0),
            h: (b - y).max(0.0),
        }
    }

    pub fn pad(self, p: f32) -> Self {
        Self {
            x: self.x - p,
            y: self.y - p,
            w: self.w + 2.0 * p,
            h: self.h + 2.0 * p,
        }
    }

    pub fn clamp_to(self, sw: f32, sh: f32) -> Self {
        let x = self.x.max(0.0).min(sw);
        let y = self.y.max(0.0).min(sh);
        let r = self.right().max(0.0).min(sw);
        let b = self.bottom().max(0.0).min(sh);
        Self {
            x,
            y,
            w: (r - x).max(0.0),
            h: (b - y).max(0.0),
        }
    }

    pub fn translate(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            w: self.w,
            h: self.h,
        }
    }

    pub fn area(self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }

    /// Pixel rect `(x, y, w, h)` covering this AABB, clamped to the image.
    pub fn to_px(self, sw: u32, sh: u32) -> Option<(u32, u32, u32, u32)> {
        let c = self.clamp_to(sw as f32, sh as f32);
        let x = c.x.floor().max(0.0) as u32;
        let y = c.y.floor().max(0.0) as u32;
        let r = c.right().ceil().min(sw as f32) as u32;
        let b = c.bottom().ceil().min(sh as f32) as u32;
        let w = r.saturating_sub(x);
        let h = b.saturating_sub(y);
        if w == 0 || h == 0 {
            None
        } else {
            Some((x.min(sw), y.min(sh), w, h))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pos_is_clone_copy_debug() {
        let p = Pos { x: 1.5, y: -2.0 };
        let p2 = p;
        assert_eq!(p.x, p2.x);
    }

    #[test]
    fn bounds_from_two_normal_order() {
        let a = Pos { x: 10.0, y: 20.0 };
        let b = Pos { x: 30.0, y: 40.0 };
        let bnd = Bounds::from_two(a, b);
        assert_eq!(bnd.x, 10.0);
        assert_eq!(bnd.y, 20.0);
        assert_eq!(bnd.w, 20.0);
        assert_eq!(bnd.h, 20.0);
    }

    #[test]
    fn bounds_from_two_reversed_order() {
        let a = Pos { x: 50.0, y: 5.0 };
        let b = Pos { x: 0.0, y: 80.0 };
        let bnd = Bounds::from_two(a, b);
        assert_eq!(bnd.x, 0.0);
        assert_eq!(bnd.y, 5.0);
        assert_eq!(bnd.w, 50.0);
        assert_eq!(bnd.h, 75.0);
    }

    #[test]
    fn bounds_from_two_zero_area() {
        let p = Pos { x: 100.0, y: 200.0 };
        let bnd = Bounds::from_two(p, p);
        assert_eq!(bnd.w, 0.0);
        assert_eq!(bnd.h, 0.0);
    }

    #[test]
    fn bounds_edges_and_corners() {
        let b = Bounds {
            x: 10.0,
            y: 20.0,
            w: 30.0,
            h: 40.0,
        };
        assert_eq!(b.right(), 40.0);
        assert_eq!(b.bottom(), 60.0);
        assert_eq!(b.center().x, 25.0);
        assert_eq!(b.center().y, 40.0);
        assert_eq!(b.nw().x, 10.0);
        assert_eq!(b.se().x, 40.0);
        assert_eq!(b.ne().y, 20.0);
        assert_eq!(b.sw().y, 60.0);
        let c = Bounds::centered(50.0, 50.0, 20.0, 10.0);
        assert_eq!(c.x, 40.0);
        assert_eq!(c.y, 45.0);
        assert_eq!(c.w, 20.0);
        assert_eq!(c.h, 10.0);
    }

    #[test]
    fn bounds_union_pad_clamp_to_px() {
        let a = Bounds {
            x: 10.0,
            y: 20.0,
            w: 10.0,
            h: 10.0,
        };
        let b = Bounds {
            x: 15.0,
            y: 5.0,
            w: 20.0,
            h: 10.0,
        };
        let u = a.union(b);
        assert_eq!(u.x, 10.0);
        assert_eq!(u.y, 5.0);
        assert_eq!(u.w, 25.0);
        assert_eq!(u.h, 25.0);
        let p = a.pad(2.0);
        assert_eq!(p.x, 8.0);
        assert_eq!(p.w, 14.0);
        let c = Bounds {
            x: -10.0,
            y: 90.0,
            w: 50.0,
            h: 30.0,
        }
        .clamp_to(100.0, 100.0);
        assert_eq!(c.x, 0.0);
        assert_eq!(c.y, 90.0);
        assert_eq!(c.w, 40.0);
        assert_eq!(c.h, 10.0);
        let (x, y, w, h) = a.to_px(100, 100).unwrap();
        assert_eq!((x, y, w, h), (10, 20, 10, 10));
        assert!((a.area() - 100.0).abs() < 0.01);
        let t = a.translate(-10.0, 5.0);
        assert_eq!(t.x, 0.0);
        assert_eq!(t.y, 25.0);
    }
}
