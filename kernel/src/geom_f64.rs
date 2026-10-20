//! Direct port of `convex/util/geometry.ts` and `geometry.test.ts`.
//! Floats stay in tests. The kernel itself uses milli-tiles.

#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Vector {
    dx: f64,
    dy: f64,
}

type Path = Vec<[f64; 5]>;

fn distance(p0: Point, p1: Point) -> f64 {
    let dx = p0.x - p1.x;
    let dy = p0.y - p1.y;
    (dx * dx + dy * dy).sqrt()
}

fn points_equal(p0: Point, p1: Point) -> bool {
    p0.x == p1.x && p0.y == p1.y
}

fn manhattan_distance(p0: Point, p1: Point) -> f64 {
    (p0.x - p1.x).abs() + (p0.y - p1.y).abs()
}

fn query_path(path: &Path, at: usize) -> (Point, Vector, f64) {
    let p = path[at];
    (Point { x: p[0], y: p[1] }, Vector { dx: p[2], dy: p[3] }, p[4])
}

fn path_overlaps(path: &Path, time: f64) -> Result<bool, String> {
    if path.len() < 2 {
        return Err(format!("Invalid path: {path:?}"));
    }
    let start = query_path(path, 0);
    let end = query_path(path, path.len() - 1);
    Ok(start.2 <= time && time <= end.2)
}

fn path_position(path: &Path, time: f64) -> Result<(Point, Vector, f64), String> {
    if path.len() < 2 {
        return Err(format!("Invalid path: {path:?}"));
    }
    let first = query_path(path, 0);
    if time < first.2 {
        return Ok((first.0, first.1, 0.0));
    }
    let last = query_path(path, path.len() - 1);
    if last.2 < time {
        return Ok((last.0, last.1, 0.0));
    }
    let mut i = 0;
    while i + 1 < path.len() {
        let segment_start = query_path(path, i);
        let segment_end = query_path(path, i + 1);
        if segment_start.2 <= time && time <= segment_end.2 {
            let interp = (time - segment_start.2) / (segment_end.2 - segment_start.2);
            let position = Point {
                x: segment_start.0.x + interp * (segment_end.0.x - segment_start.0.x),
                y: segment_start.0.y + interp * (segment_end.0.y - segment_start.0.y),
            };
            let velocity = distance(segment_start.0, segment_end.0) / (segment_end.2 - segment_start.2);
            return Ok((position, segment_start.1, velocity));
        }
        i += 1;
    }
    Err("Timestamp checks not exhaustive?".into())
}

const EPSILON: f64 = 0.0001;

fn vector(p0: Point, p1: Point) -> Vector {
    Vector { dx: p1.x - p0.x, dy: p1.y - p0.y }
}

fn vector_length(v: Vector) -> f64 {
    (v.dx * v.dx + v.dy * v.dy).sqrt()
}

fn normalize(v: Vector) -> Option<Vector> {
    let len = vector_length(v);
    if len < EPSILON {
        None
    } else {
        Some(Vector { dx: v.dx / len, dy: v.dy / len })
    }
}

fn orientation_degrees(v: Vector) -> Result<f64, String> {
    if (v.dx * v.dx + v.dy * v.dy).sqrt() < EPSILON {
        return Err(format!("Can't compute the orientation of too small vector {v:?}"));
    }
    let two_pi = 2.0 * std::f64::consts::PI;
    let radians = (v.dy.atan2(v.dx) + two_pi) % two_pi;
    Ok((radians / two_pi) * 360.0)
}

#[derive(Clone, Copy)]
struct PathComponent {
    position: Point,
    facing: Vector,
    t: f64,
}

fn pack(p: PathComponent) -> [f64; 5] {
    [p.position.x, p.position.y, p.facing.dx, p.facing.dy, p.t]
}

fn compress_path(dense: &[PathComponent]) -> Path {
    if dense.len() <= 2 {
        return dense.iter().copied().map(pack).collect();
    }
    let mut out = vec![pack(dense[0])];
    let mut last = dense[0];
    let mut candidate: Option<PathComponent> = None;
    for point in dense.iter().skip(1) {
        if candidate.is_none() {
            candidate = Some(*point);
            continue;
        }
        let cand = candidate.unwrap();
        let segment = vec![pack(last), pack(*point)];
        let (position, facing, _) = path_position(&segment, cand.t).unwrap();
        let position_close = distance(position, cand.position) < EPSILON;
        let facing_difference = Vector { dx: facing.dx - cand.facing.dx, dy: facing.dy - cand.facing.dy };
        let facing_close = vector_length(facing_difference) < EPSILON;
        if position_close && facing_close {
            candidate = Some(*point);
            continue;
        }
        out.push(pack(cand));
        last = cand;
        candidate = Some(*point);
    }
    if let Some(cand) = candidate {
        out.push(pack(cand));
    }
    out
}

/// Jest `toBeCloseTo`: absolute error under `10^-digits / 2`. Default digits is 2.
fn close_to(actual: f64, expected: f64, digits: i32) {
    let tol = 10f64.powi(-digits) / 2.0;
    assert!((actual - expected).abs() < tol, "{actual} != {expected} (tol {tol})");
}

#[test]
fn distance_cases() {
    assert_eq!(distance(Point { x: 0.0, y: 0.0 }, Point { x: 3.0, y: 4.0 }), 5.0);
    let p = Point { x: 1.0, y: 2.0 };
    assert_eq!(distance(p, p), 0.0);
    close_to(distance(Point { x: -2.0, y: -3.0 }, Point { x: 1.0, y: 2.0 }), 5.83, 2);
}

#[test]
fn points_equal_cases() {
    assert!(points_equal(Point { x: 1.0, y: 2.0 }, Point { x: 1.0, y: 2.0 }));
    assert!(!points_equal(Point { x: 3.0, y: 2.0 }, Point { x: 5.0, y: 3.0 }));
    assert!(!points_equal(Point { x: 1.0, y: 2.0 }, Point { x: 2.0, y: 2.0 }));
    assert!(!points_equal(Point { x: 1.0, y: 2.0 }, Point { x: 1.0, y: 3.0 }));
}

#[test]
fn manhattan_cases() {
    assert_eq!(manhattan_distance(Point { x: 1.0, y: 0.0 }, Point { x: 1.0, y: 2.0 }), 2.0);
    assert_eq!(manhattan_distance(Point { x: 1.0, y: 0.0 }, Point { x: 3.0, y: 2.0 }), 4.0);
    assert_eq!(manhattan_distance(Point { x: -2.0, y: 0.0 }, Point { x: 1.0, y: -2.0 }), 5.0);
    assert_eq!(manhattan_distance(Point { x: 1.0, y: 2.0 }, Point { x: 1.0, y: 2.0 }), 0.0);
}

#[test]
fn path_overlap_cases() {
    let short: Path = vec![[0.0, 0.0, 0.0, 1.0, 0.0]];
    assert!(path_overlaps(&short, 0.0).is_err());
    let path: Path = vec![[0.0, 0.0, 0.0, 1.0, 1.0], [0.0, 2.0, 0.0, 1.0, 2.0]];
    assert_eq!(path_overlaps(&path, 1.5).unwrap(), true);
    assert_eq!(path_overlaps(&path, 0.5).unwrap(), false);
    assert_eq!(path_overlaps(&path, 2.5).unwrap(), false);
}

#[test]
fn path_position_cases() {
    let short: Path = vec![[0.0, 0.0, 0.0, 1.0, 0.0]];
    assert!(path_position(&short, 0.0).is_err());
    let path: Path = vec![[1.0, 2.0, 3.0, 4.0, 2.0], [5.0, 6.0, 3.0, 4.0, 3.0]];
    let (pos, facing, vel) = path_position(&path, 1.0).unwrap();
    assert_eq!(pos, Point { x: 1.0, y: 2.0 });
    assert_eq!(facing, Vector { dx: 3.0, dy: 4.0 });
    assert_eq!(vel, 0.0);
    let (pos, facing, vel) = path_position(&path, 4.0).unwrap();
    assert_eq!(pos, Point { x: 5.0, y: 6.0 });
    assert_eq!(facing, Vector { dx: 3.0, dy: 4.0 });
    assert_eq!(vel, 0.0);
    let path: Path = vec![
        [1.0, 2.0, 7.0, 8.0, 2.0],
        [5.0, 6.0, 7.0, 8.0, 3.0],
        [10.0, 11.0, 7.0, 8.0, 4.0],
        [14.0, 15.0, 7.0, 8.0, 5.0],
    ];
    let (pos, facing, vel) = path_position(&path, 4.5).unwrap();
    assert_eq!(pos, Point { x: 12.0, y: 13.0 });
    assert_eq!(facing, Vector { dx: 7.0, dy: 8.0 });
    close_to(vel, 5.657, 2);
}

#[test]
fn vector_cases() {
    assert_eq!(vector(Point { x: 1.0, y: 2.0 }, Point { x: 2.0, y: 4.0 }), Vector { dx: 1.0, dy: 2.0 });
    assert_eq!(vector(Point { x: 1.0, y: 2.0 }, Point { x: 1.0, y: 2.0 }), Vector { dx: 0.0, dy: 0.0 });
    assert_eq!(vector(Point { x: 1.0, y: 2.0 }, Point { x: 1.0, y: 1.0 }), Vector { dx: 0.0, dy: -1.0 });
    close_to(vector_length(Vector { dx: 3.14, dy: 4.0 }), 5.09, 2);
    close_to(vector_length(Vector { dx: -3.0, dy: -4.0 }), 5.0, 2);
    close_to(vector_length(Vector { dx: 0.0, dy: 0.0 }), 0.0, 2);
    assert!(normalize(Vector { dx: 0.0, dy: 0.0 }).is_none());
    let n = normalize(Vector { dx: 3.0, dy: 4.0 }).unwrap();
    close_to(n.dx, 0.6, 2);
    close_to(n.dy, 0.8, 2);
    assert!(orientation_degrees(Vector { dx: 0.0, dy: 0.0 }).is_err());
    close_to(orientation_degrees(Vector { dx: 1.0, dy: 0.0 }).unwrap(), 0.0, 2);
    close_to(orientation_degrees(Vector { dx: 0.0, dy: 1.0 }).unwrap(), 90.0, 2);
    close_to(orientation_degrees(Vector { dx: -1.0, dy: 0.0 }).unwrap(), 180.0, 2);
    close_to(orientation_degrees(Vector { dx: 0.0, dy: -1.0 }).unwrap(), 270.0, 2);
}

#[test]
fn compress_path_cases() {
    let facing = Vector { dx: 0.0, dy: 1.0 };
    let compressed = compress_path(&[
        PathComponent { position: Point { x: 0.0, y: 0.0 }, facing, t: 0.0 },
        PathComponent { position: Point { x: 0.0, y: 1.0 }, facing, t: 1.0 },
    ]);
    assert_eq!(compressed, vec![[0.0, 0.0, 0.0, 1.0, 0.0], [0.0, 1.0, 0.0, 1.0, 1.0]]);
    let compressed = compress_path(&[
        PathComponent { position: Point { x: 0.0, y: 0.0 }, facing, t: 0.0 },
        PathComponent { position: Point { x: 0.0, y: 1.0 }, facing, t: 1.0 },
        PathComponent { position: Point { x: 0.0, y: 2.0 }, facing, t: 2.0 },
        PathComponent { position: Point { x: 0.0, y: 3.0 }, facing, t: 3.0 },
        PathComponent { position: Point { x: 0.0, y: 4.0 }, facing, t: 4.0 },
    ]);
    assert_eq!(compressed, vec![[0.0, 0.0, 0.0, 1.0, 0.0], [0.0, 4.0, 0.0, 1.0, 4.0]]);
    let up = Vector { dx: 0.0, dy: 1.0 };
    let right = Vector { dx: 1.0, dy: 0.0 };
    let compressed = compress_path(&[
        PathComponent { position: Point { x: 0.0, y: 0.0 }, facing: up, t: 0.0 },
        PathComponent { position: Point { x: 0.0, y: 1.0 }, facing: up, t: 1.0 },
        PathComponent { position: Point { x: 0.0, y: 2.0 }, facing: right, t: 2.0 },
        PathComponent { position: Point { x: 1.0, y: 2.0 }, facing: right, t: 3.0 },
        PathComponent { position: Point { x: 2.0, y: 2.0 }, facing: right, t: 4.0 },
    ]);
    assert_eq!(
        compressed,
        vec![[0.0, 0.0, 0.0, 1.0, 0.0], [0.0, 2.0, 1.0, 0.0, 2.0], [2.0, 2.0, 1.0, 0.0, 4.0]]
    );
}
