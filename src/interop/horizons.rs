use std::collections::HashMap;
use std::fmt::Display;
use lazy_static::lazy_static;
use url::form_urlencoded;

lazy_static! {
    static ref BASE_URL: String = "https://ssd.jpl.nasa.gov/api/horizons.api".into();
}

fn encode_param(param: &str) -> String {
    form_urlencoded::byte_serialize(param.as_bytes()).collect::<String>()
}

#[derive(Default)]
pub struct Request {
    pub format: Format,
    // TODO: Command, a complex thing.
    pub object_data: ObjectData,
    pub ephemeris: Ephemeris,
    pub object: Object,
}

impl Request {
    fn url(params: HashMap<String, Vec<Box<dyn ToString>>>) -> String {
        let consolidated: Vec<String> = params.iter().map(|(k, v)| {
            let multi = v.len() > 1;
            let values = v.iter().map(|item| { item.to_string() }).collect::<Vec<_>>().join(",");
            if multi {
                format!("{}='{}'", encode_param(k), encode_param(values.as_str()))
            } else {
                format!("{}={}", encode_param(k), encode_param(values.as_str()))
            }
        }).collect();
        format!("{}?{}", *BASE_URL, consolidated.join("&"))
    }
}

impl Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut params: HashMap<String, Vec<Box<dyn ToString>>> = HashMap::new();
        self.format.insert_params(&mut params);
        self.object.insert_params(&mut params);
        self.object_data.insert_params(&mut params);
        self.ephemeris.insert_params(&mut params);
        write!(f, "{}", Request::url(params))
    }
}

#[derive(Default, PartialEq)]
pub enum Format {
    JSON,
    #[default]
    Text,
}

impl Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Format::JSON => write!(f, "json"),
            Format::Text => write!(f, "text"),
        }
    }
}

impl Format {
    fn insert_params(&self, params: &mut HashMap<String, Vec<Box<dyn ToString>>>) {
        params.insert("format".to_owned(), vec![Box::new(self.to_string())]);
    }
}

#[derive(PartialEq)]
pub enum Object {
    Number(u64),
    Name(String),
    IauNumber(u64),
}

impl Default for Object {
    fn default() -> Self {
        Self::Number(399)
    }
}

impl Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Object::Number(n) => write!(f, "{}", n),
            Object::Name(n) => write!(f, "{}'", n),
            Object::IauNumber(n) => write!(f, "{}", n),
        }
    }
}

impl Object {
    fn insert_params(&self, params: &mut HashMap<String, Vec<Box<dyn ToString>>>) {
        params.insert("COMMAND".to_owned(), vec![Box::new(self.to_string())]);
    }
}

#[derive(Default, PartialEq)]
pub enum ObjectData {
    #[default]
    YES,
    NO,
}

impl Display for ObjectData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectData::YES => write!(f, "YES"),
            ObjectData::NO => write!(f, "NO"),
        }
    }
}

impl ObjectData {
    fn insert_params(&self, params: &mut HashMap<String, Vec<Box<dyn ToString>>>) {
        params.insert("OBJ_DATA".to_owned(), vec![Box::new(self.to_string())]);
    }
}

pub enum Ephemeris {
    YES(EphemerisParams),
    No,
}

impl PartialEq for Ephemeris {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Ephemeris::No, Ephemeris::No) => true,
            (Ephemeris::YES(_), Ephemeris::YES(_)) => true,
            _ => false,
        }
    }
}

impl Default for Ephemeris {
    fn default() -> Self {
        Self::YES(EphemerisParams::default())
    }
}

impl Display for Ephemeris {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ephemeris::YES(params) => write!(f, "YES"),
            Ephemeris::No => write!(f, "NO"),
        }
    }
}

impl Ephemeris {
    fn insert_params(&self, params: &mut HashMap<String, Vec<Box<dyn ToString>>>) {
        params.insert("MAKE_EPHEM".to_owned(), vec![Box::new(self.to_string())]);
        match self {
            Ephemeris::No => {},
            Ephemeris::YES(ephem) => {
                ephem.insert_params(params);
            }
        }
    }
}

pub struct EphemerisParams {
    pub ephemeris_type: EphemerisType,
    pub center: Option<Center>,
}

impl Default for EphemerisParams {
    fn default() -> Self {
        Self {
            ephemeris_type: EphemerisType::default(),
            center: Some(Center::default()),
        }
    }
}

impl EphemerisParams {
    fn insert_params(&self, params: &mut HashMap<String, Vec<Box<dyn ToString>>>) {
        self.ephemeris_type.insert_params(params);
        if let Some(center) = &self.center {
            center.insert_params(params);
        }
    }
}

/// See https://ssd-api.jpl.nasa.gov/doc/horizons.html#ephem_type
#[derive(Default, PartialEq)]
pub enum EphemerisType {
    #[default]
    Observables,
    OsculatingOrbitalElements,
    CartesianStateVectors,
    CloseApproaches,
    SpkBinaryTrajectoryFiles,
}

impl Display for EphemerisType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EphemerisType::Observables => write!(f, "OBSERVER"),
            EphemerisType::OsculatingOrbitalElements => write!(f, "ELEMENTS"),
            EphemerisType::CartesianStateVectors => write!(f, "VECTORS"),
            EphemerisType::CloseApproaches => write!(f, "APPROACH"),
            EphemerisType::SpkBinaryTrajectoryFiles => write!(f, "SPK"),
        }
    }
}

impl EphemerisType {
    fn insert_params(&self, params: &mut HashMap<String, Vec<Box<dyn ToString>>>) {
        params.insert("EPHEM_TYPE".to_owned(), vec![Box::new(self.to_string())]);
    }
}

#[derive(Default)]
pub enum Center {
    #[default]
    Geocentric,
    EarthSite(String),
    OtherSite(String, String),
    BodyCenter(String),
}

impl Display for Center {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Center::Geocentric => write!(f, "Geocentric"),
            Center::EarthSite(site) => write!(f, "{}@399", site),
            Center::OtherSite(body, site) => write!(f, "{}@{}", body, site),
            Center::BodyCenter(body) => write!(f, "500@{}", body),
        }
    }
}

impl Center {
    fn insert_params(&self, params: &mut HashMap<String, Vec<Box<dyn ToString>>>) {
        params.insert("CENTER".to_owned(), vec![Box::new(self.to_string())]);
    }
}
