//! めり込み警告(SIM-007): 立体表示の面同士が突き抜けていないかを調べる。
//!
//! 厳密な防止はしない(剛体折りの計算を止めない)。「紙が食い込んでいる」ことを
//! 見つけて警告に載せるだけの検査で、実際の紙では起こり得ない形を利用者へ知らせる。
//!
//! 判定の方針:
//! - 面を扇状に三角形へ割り、三角形の辺が相手の三角形の内部を貫くかを見る。
//!   同一平面上で重なっているだけの場合は「めり込み」としない(平らに畳んだ紙の
//!   層は必ず同一平面に重なるため。厚みは表示側のずらしで表現する)。
//! - 折り目でつながった面(頂点を2つ共有する面)は調べない。つながった2面は
//!   折り目のところで必ず接しており、完全に折り畳むと同一平面で重なるのが正常。
//! - 面数400を想定し、まず外接直方体(AABB)で組を絞ってから三角形を調べる。
//!
//! 凸でない面では扇分割の三角形が面の外へはみ出すため、まれに実際より広く
//! 見積もる(警告を出しすぎる=安全側)。

use glam::DVec3;
use ori3_model::Frame3D;

/// めり込みを見つけたときの警告文(3D表示のバッジに出る)
pub const PENETRATION_WARNING: &str = "紙が重なって食い込んでいます";

/// 幾何の許容誤差(正規化座標。長辺=1.0)。接している程度は貫通としない。
const TOL: f64 = 1e-6;

/// 立体の面同士が食い込んでいるか。
///
/// 全ての点が z≒0(平らに畳んだ生データ)のときは常に false を返す。
/// 平らな状態では全ての層が同一平面に重なるため、交差の判定に意味がない
/// (重なりの上下は層番号で表し、表示側が層をずらして見せる)。
#[must_use]
pub fn self_intersects(frame: &Frame3D) -> bool {
    let flat = frame
        .faces
        .iter()
        .all(|f| f.polygon.iter().all(|p| p[2].abs() < TOL));
    if flat {
        return false;
    }
    let parts: Vec<Part> = frame.faces.iter().map(Part::new).collect();
    for i in 0..parts.len() {
        for j in (i + 1)..parts.len() {
            let (a, b) = (&parts[i], &parts[j]);
            if !a.aabb_overlaps(b) || a.shares_edge(b) {
                continue;
            }
            if a.tris
                .iter()
                .any(|t1| b.tris.iter().any(|t2| tris_pierce(t1, t2)))
            {
                return true;
            }
        }
    }
    false
}

/// 1つの面の三角形列と外接直方体
struct Part {
    tris: Vec<[DVec3; 3]>,
    lo: DVec3,
    hi: DVec3,
    points: Vec<DVec3>,
}

impl Part {
    fn new(face: &ori3_model::Face3D) -> Part {
        let points: Vec<DVec3> = face.polygon.iter().map(|p| DVec3::from_array(*p)).collect();
        let tris = (1..points.len().saturating_sub(1))
            .map(|k| [points[0], points[k], points[k + 1]])
            .collect();
        let lo = points
            .iter()
            .copied()
            .fold(DVec3::splat(f64::MAX), DVec3::min);
        let hi = points
            .iter()
            .copied()
            .fold(DVec3::splat(f64::MIN), DVec3::max);
        Part {
            tris,
            lo,
            hi,
            points,
        }
    }

    fn aabb_overlaps(&self, other: &Part) -> bool {
        (0..3).all(|k| self.lo[k] <= other.hi[k] + TOL && other.lo[k] <= self.hi[k] + TOL)
    }

    /// 折り目でつながった面か(同じ位置の頂点を2つ以上共有する)
    fn shares_edge(&self, other: &Part) -> bool {
        let shared = self
            .points
            .iter()
            .filter(|p| other.points.iter().any(|q| (**p - *q).length() <= TOL))
            .count();
        shared >= 2
    }
}

/// 2つの三角形が突き抜けているか(辺が相手の内部を貫くか)。
/// 同一平面の重なりは貫通としない(交点の計算が退化しNoneになる)。
fn tris_pierce(t1: &[DVec3; 3], t2: &[DVec3; 3]) -> bool {
    (0..3).any(|k| segment_pierces(t1[k], t1[(k + 1) % 3], t2))
        || (0..3).any(|k| segment_pierces(t2[k], t2[(k + 1) % 3], t1))
}

/// 線分が三角形の内部を貫くか(Möller–Trumbore法)。
/// 辺・頂点にちょうど触れるだけ、線分の端でちょうど接するだけの場合は含めない
/// (紙同士が触れているのは正常なので、はっきり食い込んだ場合だけを拾う)。
fn segment_pierces(p0: DVec3, p1: DVec3, tri: &[DVec3; 3]) -> bool {
    let dir = p1 - p0;
    let (e1, e2) = (tri[1] - tri[0], tri[2] - tri[0]);
    let h = dir.cross(e2);
    let det = e1.dot(h);
    // detは長さの3乗の量なので、絶対値ではなく辺の長さに対する相対値で平行を判定する
    if det.abs() < TOL * dir.length() * e1.length() * e2.length() {
        return false; // 線分が三角形の面と平行(同一平面の重なりを含む)
    }
    let s = p0 - tri[0];
    let u = s.dot(h) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    if u <= TOL || v <= TOL || u + v >= 1.0 - TOL {
        return false; // 三角形の内部でなければ貫通としない
    }
    let t = e2.dot(q) / det;
    t > TOL && t < 1.0 - TOL
}
