use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};
use rustls_platform_verifier::ConfigVerifierExt;
use serde_json::Value;
use url::Url;

use crate::{CypherOperation, GraphDbError, GraphOperations, PushCounts};

const BOLT_MAGIC: [u8; 4] = [0x60, 0x60, 0xB0, 0x17];
const BOLT_VERSIONS: [[u8; 4]; 4] = [[0, 0, 4, 4], [0, 0, 3, 4], [0, 0, 2, 4], [0, 0, 1, 4]];
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_VALUE_DEPTH: usize = 64;

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

pub(crate) fn push(
    operations: &GraphOperations,
    uri: &str,
    user: &str,
    password: &str,
) -> Result<PushCounts, GraphDbError> {
    let endpoint = BoltEndpoint::parse(uri)?;
    let (mut stream, version) = open(&endpoint)?;
    hello(
        &mut stream,
        user,
        password,
        endpoint.routing_context.as_ref(),
    )?;
    if endpoint.routing {
        let writers = route(
            &mut stream,
            version,
            endpoint
                .routing_context
                .as_ref()
                .ok_or(GraphDbError::Protocol("missing Neo4j routing context"))?,
        )?;
        if writers
            .iter()
            .any(|address| endpoint.matches_address(address))
        {
            return execute_operations(&mut stream, operations);
        }
        let goodbye = structure(0x02, &[])?;
        let _result = write_message(&mut stream, &goodbye);
        let mut last_error = None;
        for address in writers {
            let writer = endpoint.with_address(&address)?;
            match open(&writer).and_then(|(mut stream, _version)| {
                hello(&mut stream, user, password, writer.routing_context.as_ref())?;
                execute_operations(&mut stream, operations)
            }) {
                Ok(counts) => return Ok(counts),
                Err(error) => last_error = Some(error),
            }
        }
        return Err(last_error.unwrap_or(GraphDbError::Protocol(
            "Neo4j routing table did not contain a writer",
        )));
    }
    execute_operations(&mut stream, operations)
}

fn open(endpoint: &BoltEndpoint) -> Result<(Box<dyn ReadWrite>, BoltVersion), GraphDbError> {
    let tcp = connect(&endpoint.host, endpoint.port)?;
    tcp.set_read_timeout(Some(timeout()))
        .map_err(GraphDbError::Socket)?;
    tcp.set_write_timeout(Some(timeout()))
        .map_err(GraphDbError::Socket)?;
    let mut stream: Box<dyn ReadWrite> = if endpoint.encrypted {
        Box::new(tls_stream(tcp, endpoint)?)
    } else {
        Box::new(tcp)
    };
    let version = negotiate(&mut stream)?;
    Ok((stream, version))
}

fn execute_operations(
    stream: &mut impl ReadWrite,
    operations: &GraphOperations,
) -> Result<PushCounts, GraphDbError> {
    let mut counts = PushCounts::default();
    for operation in &operations.nodes {
        execute(stream, operation)?;
        counts.nodes += 1;
    }
    for operation in &operations.edges {
        execute(stream, operation)?;
        counts.edges += 1;
    }
    let goodbye = structure(0x02, &[])?;
    let _result = write_message(stream, &goodbye);
    Ok(counts)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoltVersion {
    V44,
    V43,
    V42,
    V41,
}

fn negotiate(stream: &mut impl ReadWrite) -> Result<BoltVersion, GraphDbError> {
    stream
        .write_all(&BOLT_MAGIC)
        .map_err(GraphDbError::Socket)?;
    for version in BOLT_VERSIONS {
        stream.write_all(&version).map_err(GraphDbError::Socket)?;
    }
    stream.flush().map_err(GraphDbError::Socket)?;
    let mut selected = [0_u8; 4];
    stream
        .read_exact(&mut selected)
        .map_err(GraphDbError::Socket)?;
    match selected {
        [0, 0, 4, 4] => Ok(BoltVersion::V44),
        [0, 0, 3, 4] => Ok(BoltVersion::V43),
        [0, 0, 2, 4] => Ok(BoltVersion::V42),
        [0, 0, 1, 4] => Ok(BoltVersion::V41),
        _ => Err(GraphDbError::Protocol("Neo4j did not negotiate Bolt 4.x")),
    }
}

fn hello(
    stream: &mut impl ReadWrite,
    user: &str,
    password: &str,
    routing_context: Option<&BTreeMap<String, String>>,
) -> Result<(), GraphDbError> {
    let mut metadata = BTreeMap::from([
        (
            "user_agent".to_owned(),
            PValue::String(format!("trail/{}", env!("CARGO_PKG_VERSION"))),
        ),
        ("scheme".to_owned(), PValue::String("basic".to_owned())),
        ("principal".to_owned(), PValue::String(user.to_owned())),
        (
            "credentials".to_owned(),
            PValue::String(password.to_owned()),
        ),
    ]);
    if let Some(routing_context) = routing_context {
        metadata.insert(
            "routing".to_owned(),
            PValue::Map(string_map(routing_context)),
        );
    }
    let message = structure(0x01, &[PValue::Map(metadata)])?;
    write_message(stream, &message)?;
    expect_success(stream, "authentication")
}

fn route(
    stream: &mut impl ReadWrite,
    version: BoltVersion,
    routing_context: &BTreeMap<String, String>,
) -> Result<Vec<String>, GraphDbError> {
    let database = match version {
        BoltVersion::V44 => PValue::Map(BTreeMap::new()),
        BoltVersion::V43 => PValue::Null,
        BoltVersion::V42 | BoltVersion::V41 => {
            return Err(GraphDbError::Protocol(
                "Neo4j routing requires Bolt 4.3 or newer",
            ));
        }
    };
    let message = structure(
        0x66,
        &[
            PValue::Map(string_map(routing_context)),
            PValue::List(Vec::new()),
            database,
        ],
    )?;
    write_message(stream, &message)?;
    let mut budget = MAX_MESSAGE_BYTES;
    let response = read_response(stream, &mut budget)?;
    let PValue::Structure(0x70, fields) = response else {
        if let PValue::Structure(0x7F, fields) = response {
            return Err(failure("routing", &fields));
        }
        return Err(GraphDbError::Protocol("unexpected Neo4j routing response"));
    };
    let routing_table = fields
        .first()
        .and_then(PValue::as_map)
        .and_then(|metadata| metadata.get("rt"))
        .and_then(PValue::as_map)
        .ok_or(GraphDbError::Protocol("invalid Neo4j routing table"))?;
    let servers = routing_table
        .get("servers")
        .and_then(PValue::as_list)
        .ok_or(GraphDbError::Protocol("invalid Neo4j routing servers"))?;
    let mut writers = Vec::new();
    for server in servers {
        let Some(server) = server.as_map() else {
            continue;
        };
        if server.get("role").and_then(PValue::as_str) != Some("WRITE") {
            continue;
        }
        if let Some(addresses) = server.get("addresses").and_then(PValue::as_list) {
            writers.extend(
                addresses
                    .iter()
                    .filter_map(PValue::as_str)
                    .map(str::to_owned),
            );
        }
    }
    if writers.is_empty() {
        return Err(GraphDbError::Protocol(
            "Neo4j routing table did not contain a writer",
        ));
    }
    Ok(writers)
}

fn string_map(values: &BTreeMap<String, String>) -> BTreeMap<String, PValue> {
    values
        .iter()
        .map(|(key, value)| (key.clone(), PValue::String(value.clone())))
        .collect()
}

fn execute(stream: &mut impl ReadWrite, operation: &CypherOperation) -> Result<(), GraphDbError> {
    let params = PValue::Map(
        operation
            .params
            .iter()
            .map(|(key, value)| Ok((key.clone(), PValue::from_json(value)?)))
            .collect::<Result<_, GraphDbError>>()?,
    );
    let run = structure(
        0x10,
        &[
            PValue::String(operation.statement.clone()),
            params,
            PValue::Map(BTreeMap::new()),
        ],
    )?;
    write_message(stream, &run)?;
    let mut response_budget = MAX_MESSAGE_BYTES;
    expect_success_with_budget(stream, "query", &mut response_budget)?;
    let pull = structure(
        0x3F,
        &[PValue::Map(BTreeMap::from([(
            "n".to_owned(),
            PValue::Integer(-1),
        )]))],
    )?;
    write_message(stream, &pull)?;
    loop {
        match read_response(stream, &mut response_budget)? {
            PValue::Structure(0x70, _) => return Ok(()),
            PValue::Structure(0x71, _) => {}
            PValue::Structure(0x7F, fields) => return Err(failure("query", &fields)),
            _ => return Err(GraphDbError::Protocol("unexpected Neo4j Bolt response")),
        }
    }
}

fn expect_success(stream: &mut impl ReadWrite, stage: &'static str) -> Result<(), GraphDbError> {
    let mut budget = MAX_MESSAGE_BYTES;
    expect_success_with_budget(stream, stage, &mut budget)
}

fn expect_success_with_budget(
    stream: &mut impl ReadWrite,
    stage: &'static str,
    budget: &mut usize,
) -> Result<(), GraphDbError> {
    match read_response(stream, budget)? {
        PValue::Structure(0x70, _) => Ok(()),
        PValue::Structure(0x7F, fields) => Err(failure(stage, &fields)),
        _ => Err(GraphDbError::Protocol("unexpected Neo4j Bolt response")),
    }
}

fn failure(stage: &'static str, fields: &[PValue]) -> GraphDbError {
    let metadata = fields.first().and_then(PValue::as_map);
    let code = metadata
        .and_then(|values| values.get("code"))
        .and_then(PValue::as_str)
        .unwrap_or("Neo4jError");
    let message = metadata
        .and_then(|values| values.get("message"))
        .and_then(PValue::as_str)
        .unwrap_or("request failed");
    GraphDbError::Neo4jResponse {
        stage,
        message: format!("{}: {}", bounded(code), bounded(message)),
    }
}

fn bounded(value: &str) -> String {
    value.chars().take(1_200).collect()
}

fn write_message(stream: &mut impl Write, payload: &[u8]) -> Result<(), GraphDbError> {
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(GraphDbError::Protocol("Neo4j request exceeded safety cap"));
    }
    for chunk in payload.chunks(65_535) {
        let length = u16::try_from(chunk.len())
            .map_err(|_error| GraphDbError::Protocol("Neo4j request chunk overflow"))?;
        stream
            .write_all(&length.to_be_bytes())
            .and_then(|()| stream.write_all(chunk))
            .map_err(GraphDbError::Socket)?;
    }
    stream.write_all(&[0, 0]).map_err(GraphDbError::Socket)?;
    stream.flush().map_err(GraphDbError::Socket)
}

fn read_response(stream: &mut impl Read, budget: &mut usize) -> Result<PValue, GraphDbError> {
    let mut payload = Vec::new();
    loop {
        let mut length = [0_u8; 2];
        stream
            .read_exact(&mut length)
            .map_err(GraphDbError::Socket)?;
        let length = usize::from(u16::from_be_bytes(length));
        if length == 0 {
            break;
        }
        if length > *budget {
            return Err(GraphDbError::Protocol("Neo4j response exceeded safety cap"));
        }
        *budget -= length;
        let offset = payload.len();
        payload.resize(offset + length, 0);
        stream
            .read_exact(&mut payload[offset..])
            .map_err(GraphDbError::Socket)?;
    }
    let mut cursor = Cursor::new(payload.as_slice());
    let value = decode_value(&mut cursor, 0)?;
    if usize::try_from(cursor.position()).unwrap_or(usize::MAX) != payload.len() {
        return Err(GraphDbError::Protocol("trailing Neo4j response bytes"));
    }
    Ok(value)
}

#[derive(Clone, Debug, PartialEq)]
enum PValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    List(Vec<PValue>),
    Map(BTreeMap<String, PValue>),
    Structure(u8, Vec<PValue>),
}

impl PValue {
    fn from_json(value: &Value) -> Result<Self, GraphDbError> {
        match value {
            Value::Null => Ok(Self::Null),
            Value::Bool(value) => Ok(Self::Bool(*value)),
            Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    Ok(Self::Integer(value))
                } else if value.is_u64() {
                    Err(GraphDbError::Protocol(
                        "Neo4j integer exceeded signed 64-bit range",
                    ))
                } else {
                    value
                        .as_f64()
                        .map(Self::Float)
                        .ok_or(GraphDbError::Protocol(
                            "Neo4j number could not be represented",
                        ))
                }
            }
            Value::String(value) => Ok(Self::String(value.clone())),
            Value::Array(values) => Ok(Self::List(
                values
                    .iter()
                    .map(Self::from_json)
                    .collect::<Result<_, _>>()?,
            )),
            Value::Object(values) => Ok(Self::Map(
                values
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), Self::from_json(value)?)))
                    .collect::<Result<_, GraphDbError>>()?,
            )),
        }
    }

    fn as_map(&self) -> Option<&BTreeMap<String, PValue>> {
        match self {
            Self::Map(value) => Some(value),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&[PValue]> {
        match self {
            Self::List(value) => Some(value),
            _ => None,
        }
    }
}

fn structure(signature: u8, fields: &[PValue]) -> Result<Vec<u8>, GraphDbError> {
    if fields.len() > 15 {
        return Err(GraphDbError::Protocol("too many Neo4j structure fields"));
    }
    let mut bytes = vec![0xB0 | u8::try_from(fields.len()).unwrap_or(15), signature];
    for field in fields {
        encode_value(field, &mut bytes)?;
    }
    Ok(bytes)
}

fn encode_value(value: &PValue, output: &mut Vec<u8>) -> Result<(), GraphDbError> {
    match value {
        PValue::Null => output.push(0xC0),
        PValue::Bool(false) => output.push(0xC2),
        PValue::Bool(true) => output.push(0xC3),
        PValue::Integer(value) if (-16..=127).contains(value) => {
            output.push(i8::try_from(*value).unwrap_or_default().to_be_bytes()[0]);
        }
        PValue::Integer(value) if i8::try_from(*value).is_ok() => {
            output.extend_from_slice(&[
                0xC8,
                i8::try_from(*value).unwrap_or_default().to_be_bytes()[0],
            ]);
        }
        PValue::Integer(value) if i16::try_from(*value).is_ok() => {
            output.push(0xC9);
            output.extend_from_slice(&i16::try_from(*value).unwrap_or_default().to_be_bytes());
        }
        PValue::Integer(value) if i32::try_from(*value).is_ok() => {
            output.push(0xCA);
            output.extend_from_slice(&i32::try_from(*value).unwrap_or_default().to_be_bytes());
        }
        PValue::Integer(value) => {
            output.push(0xCB);
            output.extend_from_slice(&value.to_be_bytes());
        }
        PValue::Float(value) => {
            output.push(0xC1);
            output.extend_from_slice(&value.to_be_bytes());
        }
        PValue::String(value) => encode_string(value, output)?,
        PValue::List(values) => {
            encode_collection_header(values.len(), 0x90, 0xD4, 0xD5, 0xD6, output)?;
            for value in values {
                encode_value(value, output)?;
            }
        }
        PValue::Map(values) => {
            encode_collection_header(values.len(), 0xA0, 0xD8, 0xD9, 0xDA, output)?;
            for (key, value) in values {
                encode_string(key, output)?;
                encode_value(value, output)?;
            }
        }
        PValue::Structure(signature, fields) => {
            output.extend_from_slice(&structure(*signature, fields)?);
        }
    }
    Ok(())
}

fn encode_string(value: &str, output: &mut Vec<u8>) -> Result<(), GraphDbError> {
    encode_collection_header(value.len(), 0x80, 0xD0, 0xD1, 0xD2, output)?;
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn encode_collection_header(
    length: usize,
    tiny: u8,
    marker8: u8,
    marker16: u8,
    marker32: u8,
    output: &mut Vec<u8>,
) -> Result<(), GraphDbError> {
    if length <= 15 {
        output.push(tiny | u8::try_from(length).unwrap_or(15));
    } else if let Ok(length) = u8::try_from(length) {
        output.extend_from_slice(&[marker8, length]);
    } else if let Ok(length) = u16::try_from(length) {
        output.push(marker16);
        output.extend_from_slice(&length.to_be_bytes());
    } else {
        let length = u32::try_from(length)
            .map_err(|_error| GraphDbError::Protocol("Neo4j value exceeded protocol limit"))?;
        output.push(marker32);
        output.extend_from_slice(&length.to_be_bytes());
    }
    Ok(())
}

fn decode_value(cursor: &mut Cursor<&[u8]>, depth: usize) -> Result<PValue, GraphDbError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(GraphDbError::Protocol("Neo4j response nested too deeply"));
    }
    let marker = read_u8(cursor)?;
    match marker {
        0x00..=0x7F => Ok(PValue::Integer(i64::from(marker))),
        0xF0..=0xFF => Ok(PValue::Integer(i64::from(i8::from_be_bytes([marker])))),
        0x80..=0x8F => decode_string(cursor, usize::from(marker & 0x0F)),
        0x90..=0x9F => decode_list(cursor, usize::from(marker & 0x0F), depth),
        0xA0..=0xAF => decode_map(cursor, usize::from(marker & 0x0F), depth),
        0xB0..=0xBF => decode_structure(cursor, usize::from(marker & 0x0F), depth),
        0xC0 => Ok(PValue::Null),
        0xC1 => Ok(PValue::Float(f64::from_be_bytes(read_array(cursor)?))),
        0xC2 => Ok(PValue::Bool(false)),
        0xC3 => Ok(PValue::Bool(true)),
        0xC8 => Ok(PValue::Integer(i64::from(i8::from_be_bytes(read_array(
            cursor,
        )?)))),
        0xC9 => Ok(PValue::Integer(i64::from(i16::from_be_bytes(read_array(
            cursor,
        )?)))),
        0xCA => Ok(PValue::Integer(i64::from(i32::from_be_bytes(read_array(
            cursor,
        )?)))),
        0xCB => Ok(PValue::Integer(i64::from_be_bytes(read_array(cursor)?))),
        0xD0 => {
            let length = usize::from(read_u8(cursor)?);
            decode_string(cursor, length)
        }
        0xD1 => {
            let length = usize::from(u16::from_be_bytes(read_array(cursor)?));
            decode_string(cursor, length)
        }
        0xD2 => {
            let length =
                usize::try_from(u32::from_be_bytes(read_array(cursor)?)).unwrap_or(usize::MAX);
            decode_string(cursor, length)
        }
        0xD4 => {
            let length = usize::from(read_u8(cursor)?);
            decode_list(cursor, length, depth)
        }
        0xD5 => {
            let length = usize::from(u16::from_be_bytes(read_array(cursor)?));
            decode_list(cursor, length, depth)
        }
        0xD6 => {
            let length =
                usize::try_from(u32::from_be_bytes(read_array(cursor)?)).unwrap_or(usize::MAX);
            decode_list(cursor, length, depth)
        }
        0xD8 => {
            let length = usize::from(read_u8(cursor)?);
            decode_map(cursor, length, depth)
        }
        0xD9 => {
            let length = usize::from(u16::from_be_bytes(read_array(cursor)?));
            decode_map(cursor, length, depth)
        }
        0xDA => {
            let length =
                usize::try_from(u32::from_be_bytes(read_array(cursor)?)).unwrap_or(usize::MAX);
            decode_map(cursor, length, depth)
        }
        0xDC => {
            let length = usize::from(read_u8(cursor)?);
            decode_structure(cursor, length, depth)
        }
        0xDD => {
            let length = usize::from(u16::from_be_bytes(read_array(cursor)?));
            decode_structure(cursor, length, depth)
        }
        _ => Err(GraphDbError::Protocol(
            "unsupported Neo4j PackStream marker",
        )),
    }
}

fn decode_string(cursor: &mut Cursor<&[u8]>, length: usize) -> Result<PValue, GraphDbError> {
    let bytes = read_bytes(cursor, length)?;
    let value = std::str::from_utf8(bytes)
        .map_err(|_error| GraphDbError::Protocol("invalid UTF-8 in Neo4j response"))?;
    Ok(PValue::String(value.to_owned()))
}

fn decode_list(
    cursor: &mut Cursor<&[u8]>,
    length: usize,
    depth: usize,
) -> Result<PValue, GraphDbError> {
    validate_item_count(cursor, length)?;
    let mut values = Vec::with_capacity(length.min(1024));
    for _ in 0..length {
        values.push(decode_value(cursor, depth + 1)?);
    }
    Ok(PValue::List(values))
}

fn decode_map(
    cursor: &mut Cursor<&[u8]>,
    length: usize,
    depth: usize,
) -> Result<PValue, GraphDbError> {
    validate_item_count(cursor, length.saturating_mul(2))?;
    let mut values = BTreeMap::new();
    for _ in 0..length {
        let PValue::String(key) = decode_value(cursor, depth + 1)? else {
            return Err(GraphDbError::Protocol("non-string Neo4j map key"));
        };
        values.insert(key, decode_value(cursor, depth + 1)?);
    }
    Ok(PValue::Map(values))
}

fn decode_structure(
    cursor: &mut Cursor<&[u8]>,
    length: usize,
    depth: usize,
) -> Result<PValue, GraphDbError> {
    validate_item_count(cursor, length)?;
    let signature = read_u8(cursor)?;
    let mut fields = Vec::with_capacity(length.min(16));
    for _ in 0..length {
        fields.push(decode_value(cursor, depth + 1)?);
    }
    Ok(PValue::Structure(signature, fields))
}

fn validate_item_count(cursor: &Cursor<&[u8]>, count: usize) -> Result<(), GraphDbError> {
    let position = usize::try_from(cursor.position()).unwrap_or(usize::MAX);
    let remaining = cursor.get_ref().len().saturating_sub(position);
    if count > remaining {
        return Err(GraphDbError::Protocol("invalid Neo4j collection length"));
    }
    Ok(())
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Result<u8, GraphDbError> {
    Ok(read_array::<1>(cursor)?[0])
}

fn read_array<const N: usize>(cursor: &mut Cursor<&[u8]>) -> Result<[u8; N], GraphDbError> {
    let mut bytes = [0_u8; N];
    cursor
        .read_exact(&mut bytes)
        .map_err(|_error| GraphDbError::Protocol("truncated Neo4j response"))?;
    Ok(bytes)
}

fn read_bytes<'a>(cursor: &mut Cursor<&'a [u8]>, length: usize) -> Result<&'a [u8], GraphDbError> {
    let start = usize::try_from(cursor.position()).unwrap_or(usize::MAX);
    let end = start.saturating_add(length);
    let bytes = cursor
        .get_ref()
        .get(start..end)
        .ok_or(GraphDbError::Protocol("truncated Neo4j response"))?;
    cursor.set_position(u64::try_from(end).unwrap_or(u64::MAX));
    Ok(bytes)
}

#[derive(Clone, Debug)]
struct BoltEndpoint {
    host: String,
    port: u16,
    encrypted: bool,
    trust_any_certificate: bool,
    routing: bool,
    routing_context: Option<BTreeMap<String, String>>,
}

impl BoltEndpoint {
    fn parse(uri: &str) -> Result<Self, GraphDbError> {
        let normalized = if uri.contains("://") {
            uri.to_owned()
        } else {
            format!("bolt://{uri}")
        };
        let parsed = Url::parse(&normalized).map_err(|_error| GraphDbError::InvalidUri)?;
        if !matches!(parsed.path(), "" | "/") || parsed.fragment().is_some() {
            return Err(GraphDbError::InvalidUri);
        }
        let (encrypted, trust_any_certificate, routing) = match parsed.scheme() {
            "bolt" => (false, false, false),
            "neo4j" => (false, false, true),
            "bolt+s" => (true, false, false),
            "neo4j+s" => (true, false, true),
            "bolt+ssc" => (true, true, false),
            "neo4j+ssc" => (true, true, true),
            _ => return Err(GraphDbError::InvalidUri),
        };
        let host = parsed
            .host_str()
            .ok_or(GraphDbError::InvalidUri)?
            .to_owned();
        let port = parsed.port().unwrap_or(7687);
        let routing_context = routing.then(|| {
            let mut context = parsed
                .query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<BTreeMap<_, _>>();
            context.insert("address".to_owned(), address(&host, port));
            context
        });
        Ok(Self {
            host,
            port,
            encrypted,
            trust_any_certificate,
            routing,
            routing_context,
        })
    }

    fn with_address(&self, advertised: &str) -> Result<Self, GraphDbError> {
        let parsed = Url::parse(&format!("bolt://{advertised}"))
            .map_err(|_error| GraphDbError::Protocol("invalid Neo4j writer address"))?;
        let mut endpoint = self.clone();
        endpoint.host = parsed
            .host_str()
            .ok_or(GraphDbError::Protocol("invalid Neo4j writer address"))?
            .to_owned();
        endpoint.port = parsed.port().unwrap_or(7687);
        Ok(endpoint)
    }

    fn matches_address(&self, advertised: &str) -> bool {
        self.with_address(advertised).is_ok_and(|candidate| {
            candidate.host.eq_ignore_ascii_case(&self.host) && candidate.port == self.port
        })
    }
}

fn address(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn connect(host: &str, port: u16) -> Result<TcpStream, GraphDbError> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_error| GraphDbError::Connection("could not resolve Neo4j host"))?;
    for address in addresses {
        if let Ok(stream) = TcpStream::connect_timeout(&address, timeout()) {
            return Ok(stream);
        }
    }
    Err(GraphDbError::Connection("could not connect to Neo4j"))
}

fn tls_stream(
    stream: TcpStream,
    endpoint: &BoltEndpoint,
) -> Result<StreamOwned<ClientConnection, TcpStream>, GraphDbError> {
    let provider = rustls::crypto::ring::default_provider();
    let _installed = provider.clone().install_default();
    let config = if endpoint.trust_any_certificate {
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification {
                supported: provider.signature_verification_algorithms,
            }))
            .with_no_client_auth()
    } else {
        ClientConfig::with_platform_verifier().map_err(GraphDbError::Tls)?
    };
    let server_name =
        ServerName::try_from(endpoint.host.clone()).map_err(|_error| GraphDbError::InvalidUri)?;
    let connection =
        ClientConnection::new(Arc::new(config), server_name).map_err(GraphDbError::Tls)?;
    Ok(StreamOwned::new(connection, stream))
}

#[derive(Debug)]
struct NoCertificateVerification {
    supported: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported.supported_schemes()
    }
}

fn timeout() -> Duration {
    std::env::var("GRAPHIFY_GRAPHDB_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map_or(Duration::from_secs(30), Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn operation(statement: &str) -> CypherOperation {
        CypherOperation {
            statement: statement.to_owned(),
            params: BTreeMap::new(),
        }
    }

    #[test]
    fn bolt_client_negotiates_authenticates_and_consumes_each_write()
    -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let server = thread::spawn(move || -> std::io::Result<Vec<Vec<u8>>> {
            let (mut socket, _) = listener.accept()?;
            let mut handshake = [0_u8; 20];
            socket.read_exact(&mut handshake)?;
            if handshake[..4] != BOLT_MAGIC {
                return Err(std::io::Error::other("wrong Bolt magic"));
            }
            socket.write_all(&BOLT_VERSIONS[0])?;
            let mut messages = Vec::new();
            for _ in 0..5 {
                messages.push(read_test_message(&mut socket)?);
                socket.write_all(&[0, 3, 0xB1, 0x70, 0xA0, 0, 0])?;
            }
            Ok(messages)
        });
        let operations = GraphOperations {
            nodes: vec![operation("MERGE (n:Code {id: $id})")],
            edges: vec![operation("MATCH (a), (b) MERGE (a)-[:USES]->(b)")],
        };
        let counts = push(&operations, &format!("bolt://{address}"), "neo4j", "secret")?;
        assert_eq!(counts, PushCounts { nodes: 1, edges: 1 });
        let messages = server
            .join()
            .map_err(|_| std::io::Error::other("mock Bolt server panicked"))??;
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].get(1), Some(&0x01));
        assert_eq!(messages[1].get(1), Some(&0x10));
        assert_eq!(messages[2].get(1), Some(&0x3F));
        assert_eq!(messages[3].get(1), Some(&0x10));
        assert_eq!(messages[4].get(1), Some(&0x3F));
        Ok(())
    }

    fn read_test_message(stream: &mut impl Read) -> std::io::Result<Vec<u8>> {
        let mut payload = Vec::new();
        loop {
            let mut length = [0_u8; 2];
            stream.read_exact(&mut length)?;
            let length = usize::from(u16::from_be_bytes(length));
            if length == 0 {
                return Ok(payload);
            }
            let offset = payload.len();
            payload.resize(offset + length, 0);
            stream.read_exact(&mut payload[offset..])?;
        }
    }

    fn write_test_response(stream: &mut impl Write, value: PValue) -> Result<(), GraphDbError> {
        let response = structure(0x70, &[value])?;
        write_message(stream, &response)
    }

    #[test]
    fn neo4j_scheme_discovers_and_uses_a_write_server() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let server = thread::spawn(move || -> Result<Vec<Vec<u8>>, GraphDbError> {
            let (mut socket, _) = listener.accept().map_err(GraphDbError::Socket)?;
            let mut handshake = [0_u8; 20];
            socket
                .read_exact(&mut handshake)
                .map_err(GraphDbError::Socket)?;
            socket
                .write_all(&BOLT_VERSIONS[0])
                .map_err(GraphDbError::Socket)?;

            let mut messages = Vec::new();
            messages.push(read_test_message(&mut socket).map_err(GraphDbError::Socket)?);
            write_test_response(&mut socket, PValue::Map(BTreeMap::new()))?;

            messages.push(read_test_message(&mut socket).map_err(GraphDbError::Socket)?);
            let routing_table = PValue::Map(BTreeMap::from([(
                "rt".to_owned(),
                PValue::Map(BTreeMap::from([(
                    "servers".to_owned(),
                    PValue::List(vec![PValue::Map(BTreeMap::from([
                        ("role".to_owned(), PValue::String("WRITE".to_owned())),
                        (
                            "addresses".to_owned(),
                            PValue::List(vec![PValue::String(address.to_string())]),
                        ),
                    ]))]),
                )])),
            )]));
            write_test_response(&mut socket, routing_table)?;

            for _ in 0..2 {
                messages.push(read_test_message(&mut socket).map_err(GraphDbError::Socket)?);
                write_test_response(&mut socket, PValue::Map(BTreeMap::new()))?;
            }
            Ok(messages)
        });
        let operations = GraphOperations {
            nodes: vec![operation("MERGE (n:Code {id: $id})")],
            edges: Vec::new(),
        };
        let counts = push(
            &operations,
            &format!("neo4j://{address}?policy=primary"),
            "neo4j",
            "secret",
        )?;
        assert_eq!(counts, PushCounts { nodes: 1, edges: 0 });
        let messages = server
            .join()
            .map_err(|_| std::io::Error::other("mock Bolt server panicked"))??;
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].get(1), Some(&0x01));
        assert!(
            messages[0]
                .windows(b"policy".len())
                .any(|window| window == b"policy")
        );
        assert_eq!(messages[1].get(1), Some(&0x66));
        assert_eq!(messages[2].get(1), Some(&0x10));
        assert_eq!(messages[3].get(1), Some(&0x3F));
        Ok(())
    }

    struct Duplex {
        reads: Cursor<Vec<u8>>,
        writes: Vec<u8>,
    }

    impl Duplex {
        fn new(reads: Vec<u8>) -> Self {
            Self {
                reads: Cursor::new(reads),
                writes: Vec::new(),
            }
        }
    }

    impl Read for Duplex {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads.read(buffer)
        }
    }

    impl Write for Duplex {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.writes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn success_frame() -> Vec<u8> {
        vec![0, 3, 0xB1, 0x70, 0xA0, 0, 0]
    }

    #[test]
    fn bolt_handshake_and_parameterized_operation_follow_wire_contract() -> Result<(), GraphDbError>
    {
        let mut handshake = Duplex::new(vec![0, 0, 4, 4]);
        negotiate(&mut handshake)?;
        assert_eq!(&handshake.writes[..4], &BOLT_MAGIC);
        assert_eq!(handshake.writes.len(), 20);

        let mut hello_stream = Duplex::new(success_frame());
        hello(&mut hello_stream, "neo4j", "do-not-log", None)?;
        assert!(
            hello_stream
                .writes
                .windows(b"do-not-log".len())
                .any(|window| window == b"do-not-log")
        );

        let mut replies = success_frame();
        replies.extend_from_slice(&success_frame());
        let mut operation_stream = Duplex::new(replies);
        execute(
            &mut operation_stream,
            &CypherOperation {
                statement: "MERGE (n:Entity {id: $id}) SET n += $props".to_owned(),
                params: BTreeMap::from([
                    ("id".to_owned(), Value::String("node-1".to_owned())),
                    ("props".to_owned(), serde_json::json!({"enabled": true})),
                ]),
            },
        )?;
        assert!(
            operation_stream
                .writes
                .windows(b"node-1".len())
                .any(|window| window == b"node-1")
        );
        Ok(())
    }

    #[test]
    fn packstream_round_trips_nested_values_and_rejects_trailing_bytes() -> Result<(), GraphDbError>
    {
        let value = PValue::Map(BTreeMap::from([
            ("bool".to_owned(), PValue::Bool(true)),
            ("int".to_owned(), PValue::Integer(i64::MIN)),
            (
                "list".to_owned(),
                PValue::List(vec![PValue::Null, PValue::String("hello".to_owned())]),
            ),
        ]));
        let mut encoded = Vec::new();
        encode_value(&value, &mut encoded)?;
        let mut cursor = Cursor::new(encoded.as_slice());
        assert_eq!(decode_value(&mut cursor, 0)?, value);
        assert_eq!(
            usize::try_from(cursor.position()).unwrap_or_default(),
            encoded.len()
        );
        assert!(PValue::from_json(&Value::from(u64::MAX)).is_err());
        Ok(())
    }

    #[test]
    fn endpoint_encryption_modes_are_explicit() -> Result<(), GraphDbError> {
        let plain = BoltEndpoint::parse("bolt://localhost")?;
        assert!(!plain.encrypted);
        let secure = BoltEndpoint::parse("neo4j+s://example.test:9999")?;
        assert!(secure.encrypted);
        assert!(!secure.trust_any_certificate);
        assert_eq!(secure.port, 9999);
        let self_signed = BoltEndpoint::parse("bolt+ssc://example.test")?;
        assert!(self_signed.encrypted);
        assert!(self_signed.trust_any_certificate);
        assert!(BoltEndpoint::parse("https://example.test").is_err());
        Ok(())
    }
}
