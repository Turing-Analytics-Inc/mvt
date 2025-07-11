// encoder.rs
//
// Copyright (c) 2019-2024  Minnesota Department of Transportation
//
//! Encoder for Mapbox Vector Tile (MVT) geometry.
//!
use crate::error::{Error, Result};
use pointy::{BBox, Float, Pt, Seg, Transform};

/// Path commands
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Command {
    /// Move to new position
    MoveTo = 1,

    /// Line to new position
    LineTo = 2,

    /// Close current path
    ClosePath = 7,
}

/// Integer command
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct CommandInt {
    /// Path command
    id: Command,

    /// Command count
    count: u32,
}

/// Integer parameter
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct ParamInt {
    /// Parameter value
    value: i32,
}

/// Geometry types for [Features](struct.Feature.html).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GeomType {
    /// Point or Multipoint
    #[default]
    Point,

    /// Linestring or Multilinestring
    Linestring,

    /// Polygon or Multipolygon
    Polygon,
}

/// Encoder for [Feature](struct.Feature.html) geometry.
///
/// This can consist of Point, Linestring or Polygon data.
///
/// # Example
/// ```
/// # use mvt::{Error, GeomEncoder, GeomType};
/// # use pointy::Transform;
/// # fn main() -> Result<(), Error> {
/// let geom_data = GeomEncoder::new(GeomType::Point)
///     .point(0.0, 0.0)?
///     .point(10.0, 0.0)?
///     .encode()?;
/// # Ok(()) }
/// ```
#[derive(Default)]
pub struct GeomEncoder<F>
where
    F: Float,
{
    /// Geometry type
    geom_tp: GeomType,

    /// X,Y position at end of linestring/polygon geometry
    xy_end: Option<Pt<F>>,

    /// Transform to MVT coordinates
    transform: Transform<F>,

    /// Bounding box
    bbox: BBox<F>,

    /// Minimum X value
    x_min: i32,

    /// Maximum X value
    x_max: i32,

    /// Minimum Y value
    y_min: i32,

    /// Maximum Y value
    y_max: i32,

    /// Previous tile point
    pt0: Option<(i32, i32)>,

    /// Current tile point
    pt1: Option<(i32, i32)>,

    /// Command offset
    cmd_offset: usize,

    /// Count of geometry data
    count: u32,

    /// Encoded geometry data
    data: Vec<u32>,
}

/// Validated geometry data for [Feature](struct.Feature.html)s.
///
/// Use [GeomEncoder](struct.GeomEncoder.html) to encode.
///
/// # Example
/// ```
/// # use mvt::{Error, GeomEncoder, GeomType};
/// # use pointy::Transform;
/// # fn main() -> Result<(), Error> {
/// let geom_data = GeomEncoder::new(GeomType::Point)
///     .point(0.0, 0.0)?
///     .point(10.0, 0.0)?
///     .encode()?;
/// # Ok(()) }
/// ```
pub struct GeomData {
    /// Geometry type
    geom_tp: GeomType,

    /// Encoded geometry data
    data: Vec<u32>,
}

impl CommandInt {
    /// Create a new integer command
    fn new(id: Command, count: u32) -> Self {
        debug_assert!(count <= 0x1FFF_FFFF);
        CommandInt { id, count }
    }

    /// Encode command
    fn encode(&self) -> u32 {
        ((self.id as u32) & 0x7) | (self.count << 3)
    }

    /// Decode command
    fn decode(code: u32) -> Self {
        let id = match code & 0x7 {
            1 => Command::MoveTo,
            2 => Command::LineTo,
            7 => Command::ClosePath,
            _ => panic!("Invalid code: {code}"),
        };
        let count = code >> 3;
        CommandInt { id, count }
    }
}

impl ParamInt {
    /// Create a new integer parameter
    fn new(value: i32) -> Self {
        ParamInt { value }
    }

    /// Encode the parameter
    fn encode(&self) -> u32 {
        ((self.value << 1) ^ (self.value >> 31)) as u32
    }
}

impl<F> GeomEncoder<F>
where
    F: Float,
{
    /// Create a new geometry encoder.
    ///
    /// * `geom_tp` Geometry type.
    pub fn new(geom_tp: GeomType) -> Self {
        GeomEncoder {
            geom_tp,
            x_min: i32::MIN,
            x_max: i32::MAX,
            y_min: i32::MIN,
            y_max: i32::MAX,
            ..Default::default()
        }
    }

    /// Adjust min/max values
    fn adjust_minmax(mut self) -> Self {
        if self.bbox != BBox::default() {
            let p = self.transform * (self.bbox.x_min(), self.bbox.y_min());
            let x0 = p.x.round().to_i32().unwrap_or(i32::MIN);
            let y0 = p.y.round().to_i32().unwrap_or(i32::MIN);
            let p = self.transform * (self.bbox.x_max(), self.bbox.y_max());
            let x1 = p.x.round().to_i32().unwrap_or(i32::MAX);
            let y1 = p.y.round().to_i32().unwrap_or(i32::MAX);
            self.x_min = x0.min(x1);
            self.y_min = y0.min(y1);
            self.x_max = x0.max(x1);
            self.y_max = y0.max(y1);
        }
        self
    }

    /// Add a bounding box
    pub fn bbox(mut self, bbox: BBox<F>) -> Self {
        self.bbox = bbox;
        self.adjust_minmax()
    }

    /// Add a transform
    pub fn transform(mut self, transform: Transform<F>) -> Self {
        self.transform = transform;
        self.adjust_minmax()
    }

    /// Push a Command
    fn push_command(&mut self, cmd: Command) {
        log::trace!("push_command: {cmd:?}");
        self.cmd_offset = self.data.len();
        self.data.push(CommandInt::new(cmd, 1).encode());
    }

    /// Set count of the most recent Command.
    fn set_command_count(&mut self, count: u32) {
        let off = self.cmd_offset;
        let mut cmd = CommandInt::decode(self.data[off]);
        cmd.count = count;
        self.data[off] = cmd.encode();
    }

    /// Push one point with relative coörindates.
    fn push_point(&mut self, x: i32, y: i32) {
        log::trace!("push_point: {x},{y}");
        self.pt0 = self.pt1;
        let (px, py) = self.pt0.unwrap_or((0, 0));
        self.data.push(ParamInt::new(x.saturating_sub(px)).encode());
        self.data.push(ParamInt::new(y.saturating_sub(py)).encode());
        self.pt1 = Some((x, y));
        self.count += 1;
    }

    /// Pop most recent point.
    fn pop_point(&mut self) {
        log::trace!("pop_point");
        self.data.pop();
        self.data.pop();
        self.pt1 = self.pt0;
        self.count -= 1;
    }

    /// Add a point, taking ownership (for method chaining).
    pub fn point(mut self, x: F, y: F) -> Result<Self> {
        self.add_point(x, y)?;
        Ok(self)
    }

    /// Add a point.
    pub fn add_point(&mut self, x: F, y: F) -> Result<()> {
        self.add_boundary_points(x, y)?;
        self.add_tile_point(x, y)
    }

    /// Add one or two boundary points (if needed).
    fn add_boundary_points(&mut self, x: F, y: F) -> Result<()> {
        if let Some(pxy) = self.xy_end {
            let xy = Pt::from((x, y));
            let seg = Seg::new(pxy, xy);
            if let Some(seg) = seg.clip(self.bbox) {
                if seg.p0 != pxy {
                    self.add_tile_point(seg.p0.x, seg.p0.y)?;
                }
                if seg.p1 != xy {
                    self.add_tile_point(seg.p1.x, seg.p1.y)?;
                }
            }
        }
        match self.geom_tp {
            GeomType::Linestring | GeomType::Polygon => {
                self.xy_end = Some(Pt::from((x, y)));
            }
            _ => (),
        }
        Ok(())
    }

    /// Add a tile point.
    fn add_tile_point(&mut self, x: F, y: F) -> Result<()> {
        let pt = self.make_point(x, y)?;
        if let Some((px, py)) = self.pt1 {
            if pt.0 == px && pt.1 == py {
                if self.count == 0 {
                    // If the first point of a line in a multilinestring (or multipolygon) is the same as the last of the previous line,
                    // we skip the MoveTo command and increase the count so the next point correctly gets a LineTo.
                    self.count += 1;
                } else {
                    // Redundant points other than the first are unexpected, and entirely skipped.
                    log::trace!("redundant point: {px},{py}");
                }
                return Ok(());
            }
        }
        match self.geom_tp {
            GeomType::Point => {
                if self.count == 0 {
                    self.push_command(Command::MoveTo);
                }
            }
            GeomType::Linestring => match self.count {
                0 => self.push_command(Command::MoveTo),
                1 => self.push_command(Command::LineTo),
                _ => (),
            },
            GeomType::Polygon => {
                match self.count {
                    0 => self.push_command(Command::MoveTo),
                    1 => self.push_command(Command::LineTo),
                    _ => (),
                }
                if self.count >= 2 && self.should_simplify_point(pt.0, pt.1) {
                    self.pop_point();
                }
            }
        }
        self.push_point(pt.0, pt.1);
        Ok(())
    }

    /// Make point with tile coörindates.
    fn make_point(&self, x: F, y: F) -> Result<(i32, i32)> {
        let p = self.transform * (x, y);
        let mut x = p.x.round().to_i32().ok_or(Error::InvalidValue())?;
        let mut y = p.y.round().to_i32().ok_or(Error::InvalidValue())?;
        x = x.clamp(self.x_min, self.x_max);
        y = y.clamp(self.y_min, self.y_max);
        Ok((x, y))
    }

    /// Check if point should be simplified.
    fn should_simplify_point(&self, x: i32, y: i32) -> bool {
        if let (Some((p0x, p0y)), Some((p1x, p1y))) = (self.pt0, self.pt1) {
            if p0x == p1x && p1x == x {
                return (p0y < p1y && p1y < y) || (p0y > p1y && p1y > y);
            }
            if p0y == p1y && p1y == y {
                return (p0x < p1x && p1x < x) || (p0x > p1x && p1x > x);
            }
        }
        false
    }

    /// Complete the current geometry (for multilinestring / multipolygon).
    pub fn complete_geom(&mut self) -> Result<()> {
        // FIXME: return Error::InvalidGeometry
        //        if "MUST" rules in the spec are violated
        match self.geom_tp {
            GeomType::Point => {
                self.set_command_count(self.count);
                // early return skips geometry reset
                return Ok(());
            }
            GeomType::Linestring => {
                if self.count > 1 {
                    self.set_command_count(self.count - 1);
                }
            }
            GeomType::Polygon => {
                if self.count > 1 {
                    self.set_command_count(self.count - 1);
                    self.push_command(Command::ClosePath);
                }
            }
        }
        // reset linestring / polygon geometry state
        self.count = 0;
        self.xy_end = None;
        self.pt0 = None;
        Ok(())
    }

    /// Complete the current geometry (for multilinestring / multipolygon).
    pub fn complete(mut self) -> Result<Self> {
        self.complete_geom()?;
        Ok(self)
    }

    /// Encode the geometry data, consuming the encoder.
    pub fn encode(mut self) -> Result<GeomData> {
        // FIXME: return Error::InvalidGeometry
        //        if "MUST" rules in the spec are violated
        self = self.complete()?;
        Ok(GeomData::new(self.geom_tp, self.data))
    }
}

impl GeomData {
    /// Create new geometry data.
    ///
    /// * `geom_tp` Geometry type.
    /// * `data` Validated geometry.
    fn new(geom_tp: GeomType, data: Vec<u32>) -> Self {
        GeomData { geom_tp, data }
    }

    /// Get the geometry type
    pub(crate) fn geom_type(&self) -> GeomType {
        self.geom_tp
    }

    /// Check if data is empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Get length of data
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Get the geometry data
    pub(crate) fn into_vec(self) -> Vec<u32> {
        self.data
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // Examples from MVT spec:
    #[test]
    fn test_point() {
        let v = GeomEncoder::new(GeomType::Point)
            .point(25.0, 17.0)
            .unwrap()
            .encode()
            .unwrap()
            .into_vec();
        assert_eq!(v, vec!(9, 50, 34));
    }

    #[test]
    fn test_multipoint() {
        let v = GeomEncoder::new(GeomType::Point)
            .point(5.0, 7.0)
            .unwrap()
            .point(3.0, 2.0)
            .unwrap()
            .encode()
            .unwrap()
            .into_vec();
        assert_eq!(v, vec!(17, 10, 14, 3, 9));
    }

    #[test]
    fn test_linestring() {
        let v = GeomEncoder::new(GeomType::Linestring)
            .point(2.0, 2.0)
            .unwrap()
            .point(2.0, 10.0)
            .unwrap()
            .point(10.0, 10.0)
            .unwrap()
            .encode()
            .unwrap()
            .into_vec();
        assert_eq!(v, vec!(9, 4, 4, 18, 0, 16, 16, 0));
    }

    #[test]
    fn test_multilinestring() {
        let v = GeomEncoder::new(GeomType::Linestring)
            .point(2.0, 2.0)
            .unwrap()
            .point(2.0, 10.0)
            .unwrap()
            .point(10.0, 10.0)
            .unwrap()
            .complete()
            .unwrap()
            .point(1.0, 1.0)
            .unwrap()
            .point(3.0, 5.0)
            .unwrap()
            .encode()
            .unwrap()
            .into_vec();
        assert_eq!(v, vec!(9, 4, 4, 18, 0, 16, 16, 0, 9, 17, 17, 10, 4, 8));
    }

    #[test]
    fn test_multilinestring_with_redundant_points() {
        let v = GeomEncoder::new(GeomType::Linestring)
            .point(2.0, 2.0)
            .unwrap()
            .point(2.0, 2.0)
            .unwrap()
            .point(10.0, 10.0)
            .unwrap()
            .complete()
            .unwrap()
            .point(10.0, 10.0)
            .unwrap()
            .point(13.0, 15.0)
            .unwrap()
            .point(10.0, 10.0)
            .unwrap()
            .complete()
            .unwrap()
            .point(2.0, 2.0)
            .unwrap()
            .point(10.0, 10.0)
            .unwrap()
            .encode()
            .unwrap()
            .into_vec();
        assert_eq!(
            v,
            vec!(9, 4, 4, 10, 16, 16, 18, 6, 10, 5, 9, 9, 15, 15, 10, 16, 16)
        );
    }

    #[test]
    fn test_polygon() {
        let v = GeomEncoder::new(GeomType::Polygon)
            .point(3.0, 6.0)
            .unwrap()
            .point(8.0, 12.0)
            .unwrap()
            .point(20.0, 34.0)
            .unwrap()
            .encode()
            .unwrap()
            .into_vec();
        assert_eq!(v, vec!(9, 6, 12, 18, 10, 12, 24, 44, 15));
    }

    #[test]
    fn test_multipolygon() {
        let v = GeomEncoder::new(GeomType::Polygon)
            // positive area => exterior ring
            .point(0.0, 0.0)
            .unwrap()
            .point(10.0, 0.0)
            .unwrap()
            .point(10.0, 10.0)
            .unwrap()
            .point(0.0, 10.0)
            .unwrap()
            .complete()
            .unwrap()
            // positive area => exterior ring
            .point(11.0, 11.0)
            .unwrap()
            .point(20.0, 11.0)
            .unwrap()
            .point(20.0, 20.0)
            .unwrap()
            .point(11.0, 20.0)
            .unwrap()
            .complete()
            .unwrap()
            // negative area => interior ring
            .point(13.0, 13.0)
            .unwrap()
            .point(13.0, 17.0)
            .unwrap()
            .point(17.0, 17.0)
            .unwrap()
            .point(17.0, 13.0)
            .unwrap()
            .encode()
            .unwrap()
            .into_vec();
        assert_eq!(
            v,
            vec!(
                9, 0, 0, 26, 20, 0, 0, 20, 19, 0, 15, 9, 22, 2, 26, 18, 0, 0,
                18, 17, 0, 15, 9, 4, 13, 26, 0, 8, 8, 0, 0, 7, 15
            )
        );
    }
}
