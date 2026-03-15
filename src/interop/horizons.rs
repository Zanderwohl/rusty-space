use url::form_urlencoded;

const BASE_URL: &str = "https://ssd.jpl.nasa.gov/api/horizons.api";

// ---------------------------------------------------------------------------
// Top-level request
// ---------------------------------------------------------------------------

pub struct Request {
    pub command: Command,
    pub obj_data: bool,
    pub ephemeris: Option<EphemerisRequest>,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            command: Command::default(),
            obj_data: true,
            ephemeris: Some(EphemerisRequest::default()),
        }
    }
}

impl Request {
    pub fn to_url(&self) -> String {
        let mut params: Vec<(&str, String)> = Vec::new();
        params.push(("format", "json".into()));
        params.push(("COMMAND", format!("'{}'", self.command.api_value())));
        params.push(("OBJ_DATA", yn(self.obj_data)));
        match &self.ephemeris {
            Some(eph) => {
                params.push(("MAKE_EPHEM", "YES".into()));
                params.push(("EPHEM_TYPE", eph.api_type().into()));
                eph.append_params(&mut params);
            }
            None => {
                params.push(("MAKE_EPHEM", "NO".into()));
            }
        }
        encode_url(&params)
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();

        if !self.obj_data && self.ephemeris.is_none() {
            errors.push("No output: both OBJ_DATA and MAKE_EPHEM are off.".into());
        }

        if let Some(eph) = &self.ephemeris {
            match eph {
                EphemerisRequest::Spk(_) if !self.command.is_small_body() => {
                    errors.push("SPK files are only available for small bodies (asteroids/comets).".into());
                }
                EphemerisRequest::CloseApproach(_) if !self.command.is_small_body() => {
                    errors.push("Close-approach tables are only available for small bodies.".into());
                }
                _ => {}
            }

            if let Some(center) = eph.center() {
                if let Some(cmd_id) = self.command.body_id_string() {
                    if let Some(center_id) = center.body_id_string() {
                        if cmd_id == center_id {
                            errors.push("Observer (CENTER) cannot be the same as the target (COMMAND).".into());
                        }
                    }
                }
            }
        }

        errors
    }
}

// ---------------------------------------------------------------------------
// Command (target selection)
// ---------------------------------------------------------------------------

#[derive(PartialEq)]
pub enum Command {
    MajorBody(i64),
    SmallBodyNumber(u64),
    SmallBodyDesignation(String),
    SmallBodyName(String),
    MajorBodyList,
}

impl Default for Command {
    fn default() -> Self {
        Self::MajorBody(399)
    }
}

impl Command {
    pub fn api_value(&self) -> String {
        match self {
            Command::MajorBody(id) => format!("{}", id),
            Command::SmallBodyNumber(n) => format!("{};", n),
            Command::SmallBodyDesignation(d) => format!("DES={};", d),
            Command::SmallBodyName(name) => format!("{};", name),
            Command::MajorBodyList => "MB".into(),
        }
    }

    pub fn is_small_body(&self) -> bool {
        matches!(
            self,
            Command::SmallBodyNumber(_)
                | Command::SmallBodyDesignation(_)
                | Command::SmallBodyName(_)
        )
    }

    pub fn body_id_string(&self) -> Option<String> {
        match self {
            Command::MajorBody(id) => Some(id.to_string()),
            Command::SmallBodyNumber(n) => Some(n.to_string()),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Command::MajorBody(_) => "Major Body ID",
            Command::SmallBodyNumber(_) => "Small Body IAU#",
            Command::SmallBodyDesignation(_) => "Small Body Designation",
            Command::SmallBodyName(_) => "Small Body Name",
            Command::MajorBodyList => "List Major Bodies",
        }
    }
}

// ---------------------------------------------------------------------------
// Ephemeris request — the variant determines which params struct is active
// ---------------------------------------------------------------------------

pub enum EphemerisRequest {
    Observer(ObserverParams),
    Vectors(VectorsParams),
    Elements(ElementsParams),
    CloseApproach(CloseApproachParams),
    Spk(SpkParams),
}

impl Default for EphemerisRequest {
    fn default() -> Self {
        Self::Vectors(VectorsParams::default())
    }
}

impl EphemerisRequest {
    pub fn api_type(&self) -> &'static str {
        match self {
            Self::Observer(_) => "OBSERVER",
            Self::Vectors(_) => "VECTORS",
            Self::Elements(_) => "ELEMENTS",
            Self::CloseApproach(_) => "APPROACH",
            Self::Spk(_) => "SPK",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Observer(_) => "Observer",
            Self::Vectors(_) => "Vectors",
            Self::Elements(_) => "Elements",
            Self::CloseApproach(_) => "Close Approach",
            Self::Spk(_) => "SPK",
        }
    }

    pub fn center(&self) -> Option<&Center> {
        match self {
            Self::Observer(p) => Some(&p.center),
            Self::Vectors(p) => Some(&p.center),
            Self::Elements(p) => Some(&p.center),
            Self::CloseApproach(_) | Self::Spk(_) => None,
        }
    }

    fn append_params(&self, params: &mut Vec<(&str, String)>) {
        match self {
            Self::Observer(p) => p.append_params(params),
            Self::Vectors(p) => p.append_params(params),
            Self::Elements(p) => p.append_params(params),
            Self::CloseApproach(p) => p.append_params(params),
            Self::Spk(p) => p.append_params(params),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared enums
// ---------------------------------------------------------------------------

#[derive(Default, PartialEq, Clone, Copy)]
pub enum Center {
    #[default]
    Geocentric,
    BodyCenter(i64),
    SiteOnBody { site: i64, body: i64 },
    Coordinate { body: i64, coord_type: CoordType, lon: f64, lat: f64, alt: f64 },
}

impl Center {
    fn api_value(&self) -> String {
        match self {
            Center::Geocentric => "500@399".into(),
            Center::BodyCenter(body) => format!("500@{}", body),
            Center::SiteOnBody { site, body } => format!("{}@{}", site, body),
            Center::Coordinate { body, .. } => format!("coord@{}", body),
        }
    }

    fn append_params(&self, params: &mut Vec<(&str, String)>) {
        params.push(("CENTER", format!("'{}'", self.api_value())));
        if let Center::Coordinate { coord_type, lon, lat, alt, .. } = self {
            params.push(("COORD_TYPE", coord_type.api_value().into()));
            params.push(("SITE_COORD", format!("'{},{},{}'", lon, lat, alt)));
        }
    }

    pub fn body_id_string(&self) -> Option<String> {
        match self {
            Center::Geocentric => Some("399".into()),
            Center::BodyCenter(b) => Some(b.to_string()),
            Center::SiteOnBody { body, .. } => Some(body.to_string()),
            Center::Coordinate { body, .. } => Some(body.to_string()),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Center::Geocentric => "Geocentric",
            Center::BodyCenter(_) => "Body Center",
            Center::SiteOnBody { .. } => "Site on Body",
            Center::Coordinate { .. } => "Coordinates",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum TimeSpec {
    #[default]
    Span,
    List,
}

pub struct TimeSpan {
    pub start: String,
    pub stop: String,
    pub step: String,
}

impl Default for TimeSpan {
    fn default() -> Self {
        Self {
            start: "2025-01-01".into(),
            stop: "2025-01-02".into(),
            step: "60 min".into(),
        }
    }
}

impl TimeSpan {
    fn append_params(&self, params: &mut Vec<(&str, String)>) {
        params.push(("START_TIME", format!("'{}'", self.start)));
        params.push(("STOP_TIME", format!("'{}'", self.stop)));
        params.push(("STEP_SIZE", format!("'{}'", self.step)));
    }
}

pub struct TimeList {
    pub times: String,
    pub list_type: Option<TListType>,
}

impl Default for TimeList {
    fn default() -> Self {
        Self {
            times: "2460676.5".into(),
            list_type: Some(TListType::Jd),
        }
    }
}

impl TimeList {
    fn append_params(&self, params: &mut Vec<(&str, String)>) {
        params.push(("TLIST", format!("'{}'", self.times)));
        if let Some(lt) = &self.list_type {
            params.push(("TLIST_TYPE", lt.api_value().into()));
        }
    }
}

pub struct TimeConfig {
    pub spec: TimeSpec,
    pub span: TimeSpan,
    pub list: TimeList,
}

impl Default for TimeConfig {
    fn default() -> Self {
        Self {
            spec: TimeSpec::default(),
            span: TimeSpan::default(),
            list: TimeList::default(),
        }
    }
}

impl TimeConfig {
    fn append_params(&self, params: &mut Vec<(&str, String)>) {
        match self.spec {
            TimeSpec::Span => self.span.append_params(params),
            TimeSpec::List => self.list.append_params(params),
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum TListType {
    #[default]
    Jd,
    Mjd,
    Cal,
}

impl TListType {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Jd => "JD",
            Self::Mjd => "MJD",
            Self::Cal => "CAL",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Jd => "Julian Day",
            Self::Mjd => "Modified JD",
            Self::Cal => "Calendar",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum RefPlane {
    #[default]
    Ecliptic,
    Frame,
    BodyEquator,
}

impl RefPlane {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Ecliptic => "ECLIPTIC",
            Self::Frame => "FRAME",
            Self::BodyEquator => "BODY EQUATOR",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ecliptic => "Ecliptic",
            Self::Frame => "Frame (Equatorial)",
            Self::BodyEquator => "Body Equator",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum RefSystem {
    #[default]
    Icrf,
    B1950,
}

impl RefSystem {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Icrf => "ICRF",
            Self::B1950 => "B1950",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Icrf => "ICRF",
            Self::B1950 => "B1950",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum OutUnits {
    #[default]
    KmS,
    AuD,
    KmD,
}

impl OutUnits {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::KmS => "KM-S",
            Self::AuD => "AU-D",
            Self::KmD => "KM-D",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::KmS => "km & seconds",
            Self::AuD => "AU & days",
            Self::KmD => "km & days",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum CoordType {
    #[default]
    Geodetic,
    Cylindrical,
}

impl CoordType {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Geodetic => "GEODETIC",
            Self::Cylindrical => "CYLINDRICAL",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Geodetic => "Geodetic",
            Self::Cylindrical => "Cylindrical",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum TimeDigits {
    #[default]
    Minutes,
    Seconds,
    FracSec,
}

impl TimeDigits {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Minutes => "MINUTES",
            Self::Seconds => "SECONDS",
            Self::FracSec => "FRACSEC",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Minutes => "Minutes",
            Self::Seconds => "Seconds",
            Self::FracSec => "Fractional seconds",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum CalFormat {
    #[default]
    Cal,
    Jd,
    Both,
}

impl CalFormat {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Cal => "CAL",
            Self::Jd => "JD",
            Self::Both => "BOTH",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Cal => "Calendar",
            Self::Jd => "Julian Day",
            Self::Both => "Both",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum CalType {
    #[default]
    Mixed,
    Gregorian,
}

impl CalType {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Mixed => "MIXED",
            Self::Gregorian => "GREGORIAN",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Mixed => "Mixed Julian/Gregorian",
            Self::Gregorian => "Gregorian only",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum AngFormat {
    #[default]
    Hms,
    Deg,
}

impl AngFormat {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Hms => "HMS",
            Self::Deg => "DEG",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Hms => "HH:MM:SS",
            Self::Deg => "Degrees",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum Apparent {
    #[default]
    Airless,
    Refracted,
}

impl Apparent {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Airless => "AIRLESS",
            Self::Refracted => "REFRACTED",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Airless => "Airless",
            Self::Refracted => "Refracted",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum RangeUnits {
    #[default]
    Au,
    Km,
}

impl RangeUnits {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Au => "AU",
            Self::Km => "KM",
        }
    }
    pub fn label(&self) -> &'static str {
        self.api_value()
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum VecTable {
    Position,
    #[default]
    State,
    StateRangeRate,
    PositionRangeRate,
    Velocity,
    RangeRateOnly,
}

impl VecTable {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Position => "1",
            Self::State => "2",
            Self::StateRangeRate => "3",
            Self::PositionRangeRate => "4",
            Self::Velocity => "5",
            Self::RangeRateOnly => "6",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Position => "1 - Position {x,y,z}",
            Self::State => "2 - State {x,y,z,vx,vy,vz}",
            Self::StateRangeRate => "3 - State + light-time, range, range-rate",
            Self::PositionRangeRate => "4 - Position + light-time, range, range-rate",
            Self::Velocity => "5 - Velocity {vx,vy,vz}",
            Self::RangeRateOnly => "6 - Light-time, range, range-rate only",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum VecCorr {
    #[default]
    None,
    LightTime,
    LightTimeStellarAberr,
}

impl VecCorr {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::LightTime => "LT",
            Self::LightTimeStellarAberr => "LT+S",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "None (geometric)",
            Self::LightTime => "Light-time",
            Self::LightTimeStellarAberr => "Light-time + stellar aberration",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum TpType {
    #[default]
    Absolute,
    Relative,
}

impl TpType {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Absolute => "ABSOLUTE",
            Self::Relative => "RELATIVE",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Absolute => "Absolute",
            Self::Relative => "Relative",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum CaTableType {
    #[default]
    Standard,
    Extended,
}

impl CaTableType {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Standard => "STANDARD",
            Self::Extended => "EXTENDED",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Extended => "Extended",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum ObserverTimeType {
    #[default]
    Ut,
    Tt,
}

impl ObserverTimeType {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Ut => "UT",
            Self::Tt => "TT",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ut => "UT",
            Self::Tt => "TT",
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum VectorTimeType {
    #[default]
    Tdb,
    Ut,
}

impl VectorTimeType {
    pub fn api_value(&self) -> &'static str {
        match self {
            Self::Tdb => "TDB",
            Self::Ut => "UT",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Tdb => "TDB",
            Self::Ut => "UT",
        }
    }
}

// ---------------------------------------------------------------------------
// Per-type parameter structs
// ---------------------------------------------------------------------------

pub struct ObserverParams {
    pub center: Center,
    pub time: TimeConfig,
    pub quantities: String,
    pub ref_plane: RefPlane,
    pub ref_system: RefSystem,
    pub time_digits: TimeDigits,
    pub time_type: ObserverTimeType,
    pub cal_format: CalFormat,
    pub cal_type: CalType,
    pub ang_format: AngFormat,
    pub apparent: Apparent,
    pub range_units: RangeUnits,
    pub suppress_range_rate: bool,
    pub extra_prec: bool,
    pub csv_format: bool,
    pub r_t_s_only: bool,
    pub skip_daylight: bool,
}

impl Default for ObserverParams {
    fn default() -> Self {
        Self {
            center: Center::default(),
            time: TimeConfig::default(),
            quantities: "1,9,20,23,24,29".into(),
            ref_plane: RefPlane::default(),
            ref_system: RefSystem::default(),
            time_digits: TimeDigits::default(),
            time_type: ObserverTimeType::default(),
            cal_format: CalFormat::default(),
            cal_type: CalType::default(),
            ang_format: AngFormat::default(),
            apparent: Apparent::default(),
            range_units: RangeUnits::default(),
            suppress_range_rate: false,
            extra_prec: false,
            csv_format: false,
            r_t_s_only: false,
            skip_daylight: false,
        }
    }
}

impl ObserverParams {
    fn append_params(&self, params: &mut Vec<(&str, String)>) {
        self.center.append_params(params);
        self.time.append_params(params);
        params.push(("QUANTITIES", format!("'{}'", self.quantities)));
        params.push(("REF_PLANE", self.ref_plane.api_value().into()));
        params.push(("REF_SYSTEM", self.ref_system.api_value().into()));
        params.push(("TIME_DIGITS", self.time_digits.api_value().into()));
        params.push(("TIME_TYPE", self.time_type.api_value().into()));
        params.push(("CAL_FORMAT", self.cal_format.api_value().into()));
        params.push(("CAL_TYPE", self.cal_type.api_value().into()));
        params.push(("ANG_FORMAT", self.ang_format.api_value().into()));
        params.push(("APPARENT", self.apparent.api_value().into()));
        params.push(("RANGE_UNITS", self.range_units.api_value().into()));
        params.push(("SUPPRESS_RANGE_RATE", yn(self.suppress_range_rate)));
        params.push(("EXTRA_PREC", yn(self.extra_prec)));
        params.push(("CSV_FORMAT", yn(self.csv_format)));
        params.push(("R_T_S_ONLY", yn(self.r_t_s_only)));
        params.push(("SKIP_DAYLT", yn(self.skip_daylight)));
    }
}

pub struct VectorsParams {
    pub center: Center,
    pub time: TimeConfig,
    pub ref_plane: RefPlane,
    pub ref_system: RefSystem,
    pub out_units: OutUnits,
    pub vec_table: VecTable,
    pub vec_corr: VecCorr,
    pub vec_labels: bool,
    pub vec_delta_t: bool,
    pub csv_format: bool,
    pub cal_type: CalType,
    pub time_digits: TimeDigits,
    pub time_type: VectorTimeType,
}

impl Default for VectorsParams {
    fn default() -> Self {
        Self {
            center: Center::BodyCenter(10),
            time: TimeConfig::default(),
            ref_plane: RefPlane::Ecliptic,
            ref_system: RefSystem::default(),
            out_units: OutUnits::KmS,
            vec_table: VecTable::State,
            vec_corr: VecCorr::default(),
            vec_labels: true,
            vec_delta_t: false,
            csv_format: false,
            cal_type: CalType::default(),
            time_digits: TimeDigits::default(),
            time_type: VectorTimeType::default(),
        }
    }
}

impl VectorsParams {
    fn append_params(&self, params: &mut Vec<(&str, String)>) {
        self.center.append_params(params);
        self.time.append_params(params);
        params.push(("REF_PLANE", self.ref_plane.api_value().into()));
        params.push(("REF_SYSTEM", self.ref_system.api_value().into()));
        params.push(("OUT_UNITS", self.out_units.api_value().into()));
        params.push(("VEC_TABLE", format!("'{}'", self.vec_table.api_value())));
        params.push(("VEC_CORR", format!("'{}'", self.vec_corr.api_value())));
        params.push(("VEC_LABELS", yn(self.vec_labels)));
        params.push(("VEC_DELTA_T", yn(self.vec_delta_t)));
        params.push(("CSV_FORMAT", yn(self.csv_format)));
        params.push(("CAL_TYPE", self.cal_type.api_value().into()));
        params.push(("TIME_DIGITS", self.time_digits.api_value().into()));
        params.push(("TIME_TYPE", self.time_type.api_value().into()));
    }
}

pub struct ElementsParams {
    pub center: Center,
    pub time: TimeConfig,
    pub ref_system: RefSystem,
    pub out_units: OutUnits,
    pub elm_labels: bool,
    pub tp_type: TpType,
    pub csv_format: bool,
    pub cal_type: CalType,
    pub time_digits: TimeDigits,
}

impl Default for ElementsParams {
    fn default() -> Self {
        Self {
            center: Center::BodyCenter(10),
            time: TimeConfig::default(),
            ref_system: RefSystem::default(),
            out_units: OutUnits::AuD,
            elm_labels: true,
            tp_type: TpType::default(),
            csv_format: false,
            cal_type: CalType::default(),
            time_digits: TimeDigits::default(),
        }
    }
}

impl ElementsParams {
    fn append_params(&self, params: &mut Vec<(&str, String)>) {
        self.center.append_params(params);
        self.time.append_params(params);
        params.push(("REF_SYSTEM", self.ref_system.api_value().into()));
        params.push(("OUT_UNITS", self.out_units.api_value().into()));
        params.push(("ELM_LABELS", yn(self.elm_labels)));
        params.push(("TP_TYPE", self.tp_type.api_value().into()));
        params.push(("CSV_FORMAT", yn(self.csv_format)));
        params.push(("CAL_TYPE", self.cal_type.api_value().into()));
        params.push(("TIME_DIGITS", self.time_digits.api_value().into()));
    }
}

pub struct CloseApproachParams {
    pub ca_table_type: CaTableType,
    pub cal_type: CalType,
}

impl Default for CloseApproachParams {
    fn default() -> Self {
        Self {
            ca_table_type: CaTableType::default(),
            cal_type: CalType::default(),
        }
    }
}

impl CloseApproachParams {
    fn append_params(&self, params: &mut Vec<(&str, String)>) {
        params.push(("CA_TABLE_TYPE", self.ca_table_type.api_value().into()));
        params.push(("CAL_TYPE", self.cal_type.api_value().into()));
    }
}

pub struct SpkParams {
    pub start_time: String,
    pub stop_time: String,
}

impl Default for SpkParams {
    fn default() -> Self {
        Self {
            start_time: "2025-01-01".into(),
            stop_time: "2025-01-02".into(),
        }
    }
}

impl SpkParams {
    fn append_params(&self, params: &mut Vec<(&str, String)>) {
        params.push(("START_TIME", format!("'{}'", self.start_time)));
        params.push(("STOP_TIME", format!("'{}'", self.stop_time)));
    }
}

// ---------------------------------------------------------------------------
// Ephemeris type tag (used by the UI to switch types while preserving state)
// ---------------------------------------------------------------------------

#[derive(PartialEq, Clone, Copy)]
pub enum EphemerisTypeTag {
    Observer,
    Vectors,
    Elements,
    CloseApproach,
    Spk,
}

impl EphemerisTypeTag {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Observer => "Observer",
            Self::Vectors => "Vectors",
            Self::Elements => "Elements",
            Self::CloseApproach => "Close Approach",
            Self::Spk => "SPK",
        }
    }

    pub const ALL: [EphemerisTypeTag; 5] = [
        Self::Observer,
        Self::Vectors,
        Self::Elements,
        Self::CloseApproach,
        Self::Spk,
    ];
}

impl EphemerisRequest {
    pub fn tag(&self) -> EphemerisTypeTag {
        match self {
            Self::Observer(_) => EphemerisTypeTag::Observer,
            Self::Vectors(_) => EphemerisTypeTag::Vectors,
            Self::Elements(_) => EphemerisTypeTag::Elements,
            Self::CloseApproach(_) => EphemerisTypeTag::CloseApproach,
            Self::Spk(_) => EphemerisTypeTag::Spk,
        }
    }

    pub fn switch_to(&mut self, tag: EphemerisTypeTag) {
        if self.tag() == tag {
            return;
        }
        *self = match tag {
            EphemerisTypeTag::Observer => Self::Observer(ObserverParams::default()),
            EphemerisTypeTag::Vectors => Self::Vectors(VectorsParams::default()),
            EphemerisTypeTag::Elements => Self::Elements(ElementsParams::default()),
            EphemerisTypeTag::CloseApproach => Self::CloseApproach(CloseApproachParams::default()),
            EphemerisTypeTag::Spk => Self::Spk(SpkParams::default()),
        };
    }
}

// ---------------------------------------------------------------------------
// URL encoding helpers
// ---------------------------------------------------------------------------

fn yn(b: bool) -> String {
    if b { "YES".into() } else { "NO".into() }
}

fn encode_url(params: &[(&str, String)]) -> String {
    let pairs: Vec<String> = params
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                form_urlencoded::byte_serialize(k.as_bytes()).collect::<String>(),
                form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>(),
            )
        })
        .collect();
    format!("{}?{}", BASE_URL, pairs.join("&"))
}
