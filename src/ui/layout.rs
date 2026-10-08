use crate::render::Rect;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Pane {
    pub id: u64,
    pub tabs: Vec<u64>,
    pub active: usize,
}
#[derive(Clone)]
pub struct Divider {
    pub path: Vec<bool>,
    pub strip: Rect,
    pub parent: Rect,
    pub down: bool,
}
const GAP: u32 = 2;
fn children(rect: Rect, down: bool, ratio: f32) -> (Rect, Rect) {
    let ratio = ratio.clamp(0.15, 0.85);
    if down {
        let height = (rect.height as f32 * ratio) as u32;
        (
            Rect {
                height: height.saturating_sub(GAP),
                ..rect
            },
            Rect {
                y: rect.y + (height + GAP) as i32,
                height: rect.height.saturating_sub(height + GAP),
                ..rect
            },
        )
    } else {
        let width = (rect.width as f32 * ratio) as u32;
        (
            Rect {
                width: width.saturating_sub(GAP),
                ..rect
            },
            Rect {
                x: rect.x + (width + GAP) as i32,
                width: rect.width.saturating_sub(width + GAP),
                ..rect
            },
        )
    }
}
impl Pane {
    pub fn tab(&self) -> Option<u64> {
        self.tabs.get(self.active).copied()
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub enum Split {
    Pane(Pane),
    Branch {
        down: bool,
        ratio: f32,
        first: Box<Split>,
        second: Box<Split>,
    },
}
impl Split {
    pub fn pane(&self, id: u64) -> Option<&Pane> {
        match self {
            Self::Pane(p) if p.id == id => Some(p),
            Self::Pane(_) => None,
            Self::Branch { first, second, .. } => first.pane(id).or_else(|| second.pane(id)),
        }
    }
    pub fn pane_mut(&mut self, id: u64) -> Option<&mut Pane> {
        match self {
            Self::Pane(p) if p.id == id => Some(p),
            Self::Pane(_) => None,
            Self::Branch { first, second, .. } => {
                first.pane_mut(id).or_else(|| second.pane_mut(id))
            }
        }
    }
    pub fn split(&mut self, id: u64, new: Pane, down: bool) -> bool {
        self.split_at(id, new, down, false)
    }
    /// Splits pane `id`; with `before` the new pane goes left of / above it.
    pub fn split_at(&mut self, id: u64, new: Pane, down: bool, before: bool) -> bool {
        match self {
            Self::Pane(p) if p.id == id => {
                let old = Box::new(Self::Pane(p.clone()));
                let new = Box::new(Self::Pane(new));
                let (first, second) = if before { (new, old) } else { (old, new) };
                *self = Self::Branch { down, ratio: 0.5, first, second };
                true
            }
            Self::Branch { first, second, .. } => {
                first.split_at(id, new.clone(), down, before) || second.split_at(id, new, down, before)
            }
            _ => false,
        }
    }
    pub fn remove_empty(&mut self) {
        if let Self::Branch { first, second, .. } = self {
            first.remove_empty();
            second.remove_empty();
            if matches!(&**first, Self::Pane(p) if p.tabs.is_empty()) {
                *self = (**second).clone();
            } else if matches!(&**second, Self::Pane(p) if p.tabs.is_empty()) {
                *self = (**first).clone();
            }
        }
    }
    pub fn leaves(&self, rect: Rect, output: &mut Vec<(u64, Rect)>) {
        match self {
            Self::Pane(p) => output.push((p.id, rect)),
            Self::Branch {
                down,
                ratio,
                first,
                second,
            } => {
                let (a, b) = children(rect, *down, *ratio);
                first.leaves(a, output);
                second.leaves(b, output);
            }
        }
    }
    /// Draggable divider strips, each with the path of child choices that leads to its branch.
    pub fn dividers(&self, rect: Rect, path: &mut Vec<bool>, out: &mut Vec<Divider>) {
        if let Self::Branch {
            down,
            ratio,
            first,
            second,
        } = self
        {
            let (a, b) = children(rect, *down, *ratio);
            let strip = if *down {
                Rect {
                    y: a.y + a.height as i32 - 3,
                    height: b.y.saturating_sub(a.y + a.height as i32) as u32 + 6,
                    ..rect
                }
            } else {
                Rect {
                    x: a.x + a.width as i32 - 3,
                    width: b.x.saturating_sub(a.x + a.width as i32) as u32 + 6,
                    ..rect
                }
            };
            out.push(Divider {
                path: path.clone(),
                strip,
                parent: rect,
                down: *down,
            });
            path.push(false);
            first.dividers(a, path, out);
            path.pop();
            path.push(true);
            second.dividers(b, path, out);
            path.pop();
        }
    }
    pub fn ratio_mut(&mut self, path: &[bool]) -> Option<&mut f32> {
        match (self, path.split_first()) {
            (Self::Branch { ratio, .. }, None) => Some(ratio),
            (Self::Branch { first, second, .. }, Some((&to_second, rest))) => {
                if to_second { second } else { first }.ratio_mut(rest)
            }
            _ => None,
        }
    }
    pub fn first_pane(&self) -> u64 {
        match self {
            Self::Pane(p) => p.id,
            Self::Branch { first, .. } => first.first_pane(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_splits_collapse_without_losing_tabs() {
        let mut tree = Split::Pane(Pane {
            id: 1,
            tabs: vec![10],
            active: 0,
        });
        assert!(tree.split(
            1,
            Pane {
                id: 2,
                tabs: vec![20],
                active: 0
            },
            false
        ));
        assert!(tree.split(
            2,
            Pane {
                id: 3,
                tabs: vec![30],
                active: 0
            },
            true
        ));
        tree.pane_mut(2).unwrap().tabs.clear();
        tree.remove_empty();
        assert!(tree.pane(2).is_none());
        assert_eq!(tree.pane(3).unwrap().tab(), Some(30));
    }
    #[test]
    fn dividers_resize_by_path() {
        let pane = |id| Pane { id, tabs: vec![id], active: 0 };
        let mut tree = Split::Pane(pane(1));
        tree.split(1, pane(2), false);
        tree.split(2, pane(3), true);
        let window = Rect { x: 0, y: 0, width: 1000, height: 600 };
        let mut dividers = Vec::new();
        tree.dividers(window, &mut Vec::new(), &mut dividers);
        assert_eq!(dividers.len(), 2);
        let inner = dividers.iter().find(|d| d.down).unwrap();
        assert_eq!(inner.path, vec![true]);
        *tree.ratio_mut(&inner.path).unwrap() = 0.25;
        let mut leaves = Vec::new();
        tree.leaves(window, &mut leaves);
        assert!(leaves[1].1.height < leaves[2].1.height);
    }
}
