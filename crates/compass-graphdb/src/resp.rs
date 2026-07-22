use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use url::Url;

use crate::{GraphDbError, GraphOperations, PushCounts, falkordb_query};

const DEFAULT_PORT: u16 = 6379;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_RESPONSE_DEPTH: usize = 32;

pub(crate) fn push(
    operations: &GraphOperations,
    uri: &str,
    user: Option<&str>,
    password: Option<&str>,
    graph_name: &str,
) -> Result<PushCounts, GraphDbError> {
    let endpoint = FalkorEndpoint::parse(uri, user, password)?;
    let stream = connect(&endpoint.host, endpoint.port)?;
    stream
        .set_read_timeout(Some(timeout()))
        .map_err(GraphDbError::Socket)?;
    stream
        .set_write_timeout(Some(timeout()))
        .map_err(GraphDbError::Socket)?;
    let mut stream = BufReader::new(stream);
    if let Some(password) = endpoint.password.as_deref() {
        let mut command = vec!["AUTH".to_owned()];
        if let Some(user) = endpoint.user.as_deref() {
            command.push(user.to_owned());
        }
        command.push(password.to_owned());
        run_command(&mut stream, &command, "authentication")?;
    }
    let mut counts = PushCounts::default();
    for operation in &operations.nodes {
        graph_query(&mut stream, graph_name, operation)?;
        counts.nodes += 1;
    }
    for operation in &operations.edges {
        graph_query(&mut stream, graph_name, operation)?;
        counts.edges += 1;
    }
    Ok(counts)
}

fn graph_query(
    stream: &mut BufReader<TcpStream>,
    graph_name: &str,
    operation: &crate::CypherOperation,
) -> Result<(), GraphDbError> {
    run_command(
        stream,
        &[
            "GRAPH.QUERY".to_owned(),
            graph_name.to_owned(),
            falkordb_query(operation),
        ],
        "query",
    )
}

fn run_command(
    stream: &mut BufReader<TcpStream>,
    command: &[String],
    stage: &'static str,
) -> Result<(), GraphDbError> {
    let encoded = encode_command(command);
    stream
        .get_mut()
        .write_all(&encoded)
        .and_then(|()| stream.get_mut().flush())
        .map_err(GraphDbError::Socket)?;
    let mut budget = MAX_RESPONSE_BYTES;
    match read_value(stream, 0, &mut budget)? {
        RespValue::Error(message) => Err(GraphDbError::FalkorResponse {
            stage,
            message: bounded_message(&message),
        }),
        _ => Ok(()),
    }
}

fn encode_command(command: &[String]) -> Vec<u8> {
    let mut encoded = format!("*{}\r\n", command.len()).into_bytes();
    for argument in command {
        encoded.extend_from_slice(format!("${}\r\n", argument.len()).as_bytes());
        encoded.extend_from_slice(argument.as_bytes());
        encoded.extend_from_slice(b"\r\n");
    }
    encoded
}

#[derive(Debug)]
enum RespValue {
    Scalar,
    Error(String),
}

fn read_value(
    reader: &mut impl BufRead,
    depth: usize,
    budget: &mut usize,
) -> Result<RespValue, GraphDbError> {
    if depth > MAX_RESPONSE_DEPTH {
        return Err(GraphDbError::Protocol(
            "FalkorDB response nested too deeply",
        ));
    }
    let marker = read_byte(reader, budget)?;
    match marker {
        b'+' | b':' | b',' | b'_' | b'#' | b'(' => {
            let _line = read_line(reader, budget)?;
            Ok(RespValue::Scalar)
        }
        b'-' => Ok(RespValue::Error(read_line(reader, budget)?)),
        b'!' => {
            let bytes = read_bulk(reader, budget)?
                .ok_or(GraphDbError::Protocol("null FalkorDB error response"))?;
            Ok(RespValue::Error(
                String::from_utf8_lossy(&bytes).into_owned(),
            ))
        }
        b'$' | b'=' => {
            let _bytes = read_bulk(reader, budget)?;
            Ok(RespValue::Scalar)
        }
        b'*' | b'~' | b'>' => {
            let count = collection_length(&read_line(reader, budget)?, *budget)?;
            for _ in 0..count {
                if let RespValue::Error(message) = read_value(reader, depth + 1, budget)? {
                    return Ok(RespValue::Error(message));
                }
            }
            Ok(RespValue::Scalar)
        }
        b'%' => {
            let count = collection_length(&read_line(reader, budget)?, *budget)?
                .checked_mul(2)
                .ok_or(GraphDbError::Protocol("invalid FalkorDB map length"))?;
            for _ in 0..count {
                if let RespValue::Error(message) = read_value(reader, depth + 1, budget)? {
                    return Ok(RespValue::Error(message));
                }
            }
            Ok(RespValue::Scalar)
        }
        _ => Err(GraphDbError::Protocol("invalid FalkorDB response marker")),
    }
}

fn read_bulk(
    reader: &mut impl BufRead,
    budget: &mut usize,
) -> Result<Option<Vec<u8>>, GraphDbError> {
    let length = parse_length(&read_line(reader, budget)?)?;
    if length == -1 {
        return Ok(None);
    }
    let length = usize::try_from(length)
        .map_err(|_error| GraphDbError::Protocol("invalid FalkorDB response length"))?;
    let total = length
        .checked_add(2)
        .ok_or(GraphDbError::Protocol("invalid FalkorDB response length"))?;
    let mut bytes = read_exact_bounded(reader, total, budget)?;
    if !bytes.ends_with(b"\r\n") {
        return Err(GraphDbError::Protocol(
            "unterminated FalkorDB bulk response",
        ));
    }
    bytes.truncate(length);
    Ok(Some(bytes))
}

fn collection_length(value: &str, budget: usize) -> Result<usize, GraphDbError> {
    let length = parse_length(value)?;
    if length == -1 {
        return Ok(0);
    }
    let count = usize::try_from(length)
        .map_err(|_error| GraphDbError::Protocol("invalid FalkorDB response length"))?;
    if count > budget / 3 {
        return Err(GraphDbError::Protocol(
            "FalkorDB response exceeded safety cap",
        ));
    }
    Ok(count)
}

fn read_byte(reader: &mut impl Read, budget: &mut usize) -> Result<u8, GraphDbError> {
    let bytes = read_exact_bounded(reader, 1, budget)?;
    bytes
        .first()
        .copied()
        .ok_or(GraphDbError::Protocol("truncated FalkorDB response"))
}

fn read_line(reader: &mut impl BufRead, budget: &mut usize) -> Result<String, GraphDbError> {
    let mut bytes = Vec::new();
    let read = reader
        .take(u64::try_from((*budget).min(64 * 1024)).unwrap_or(64 * 1024))
        .read_until(b'\n', &mut bytes)
        .map_err(GraphDbError::Socket)?;
    *budget = budget.saturating_sub(read);
    if !bytes.ends_with(b"\r\n") {
        return Err(GraphDbError::Protocol("unterminated FalkorDB response"));
    }
    bytes.truncate(bytes.len().saturating_sub(2));
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_exact_bounded(
    reader: &mut impl Read,
    length: usize,
    budget: &mut usize,
) -> Result<Vec<u8>, GraphDbError> {
    if length > *budget {
        return Err(GraphDbError::Protocol(
            "FalkorDB response exceeded safety cap",
        ));
    }
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .map_err(GraphDbError::Socket)?;
    *budget -= length;
    Ok(bytes)
}

fn parse_length(value: &str) -> Result<i64, GraphDbError> {
    value
        .parse::<i64>()
        .map_err(|_error| GraphDbError::Protocol("invalid FalkorDB response length"))
}

fn bounded_message(message: &str) -> String {
    message.chars().take(1_200).collect()
}

struct FalkorEndpoint {
    host: String,
    port: u16,
    user: Option<String>,
    password: Option<String>,
}

impl FalkorEndpoint {
    fn parse(uri: &str, user: Option<&str>, password: Option<&str>) -> Result<Self, GraphDbError> {
        let normalized = if uri.contains("://") {
            uri.to_owned()
        } else {
            format!("redis://{uri}")
        };
        let parsed = Url::parse(&normalized).map_err(|_error| GraphDbError::InvalidUri)?;
        let host = parsed.host_str().unwrap_or("localhost").to_owned();
        let uri_user = (!parsed.username().is_empty()).then(|| parsed.username().to_owned());
        let uri_password = parsed.password().map(str::to_owned);
        let effective_password = uri_password.or_else(|| {
            password
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        });
        let effective_user = uri_user.or_else(|| {
            effective_password
                .as_ref()
                .and_then(|_| user.filter(|value| !value.is_empty()).map(str::to_owned))
        });
        Ok(Self {
            host,
            port: parsed.port().unwrap_or(DEFAULT_PORT),
            user: effective_user,
            password: effective_password,
        })
    }
}

fn connect(host: &str, port: u16) -> Result<TcpStream, GraphDbError> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_error| GraphDbError::Connection("could not resolve FalkorDB host"))?;
    for address in addresses {
        if let Ok(stream) = TcpStream::connect_timeout(&address, timeout()) {
            return Ok(stream);
        }
    }
    Err(GraphDbError::Connection("could not connect to FalkorDB"))
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
    use crate::CypherOperation;
    use std::collections::BTreeMap;
    use std::net::TcpListener;
    use std::thread;

    fn operation(statement: &str) -> CypherOperation {
        CypherOperation {
            statement: statement.to_owned(),
            params: BTreeMap::new(),
        }
    }

    #[test]
    fn resp_client_authenticates_and_runs_one_query_per_fact()
    -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let server = thread::spawn(move || -> std::io::Result<Vec<Vec<String>>> {
            let (socket, _) = listener.accept()?;
            let mut socket = BufReader::new(socket);
            let mut commands = Vec::new();
            for _ in 0..3 {
                commands.push(read_test_command(&mut socket)?);
                socket.get_mut().write_all(b"+OK\r\n")?;
            }
            Ok(commands)
        });
        let operations = GraphOperations {
            nodes: vec![operation("MERGE (n:Code {id: $id})")],
            edges: vec![operation("MATCH (a), (b) MERGE (a)-[:USES]->(b)")],
        };
        let counts = push(
            &operations,
            &format!("falkordb://compass:secret@{address}"),
            Some("ignored"),
            Some("ignored"),
            "graphify",
        )?;
        assert_eq!(counts, PushCounts { nodes: 1, edges: 1 });
        let commands = server
            .join()
            .map_err(|_| std::io::Error::other("mock RESP server panicked"))??;
        assert_eq!(commands[0], ["AUTH", "compass", "secret"]);
        assert_eq!(commands[1].first().map(String::as_str), Some("GRAPH.QUERY"));
        assert_eq!(commands[2].first().map(String::as_str), Some("GRAPH.QUERY"));
        Ok(())
    }

    fn read_test_command(reader: &mut impl BufRead) -> std::io::Result<Vec<String>> {
        let count = read_test_line(reader)?
            .strip_prefix('*')
            .ok_or_else(|| std::io::Error::other("missing RESP array"))?
            .parse::<usize>()
            .map_err(std::io::Error::other)?;
        let mut command = Vec::new();
        for _ in 0..count {
            let length = read_test_line(reader)?
                .strip_prefix('$')
                .ok_or_else(|| std::io::Error::other("missing RESP bulk string"))?
                .parse::<usize>()
                .map_err(std::io::Error::other)?;
            let mut bytes = vec![0_u8; length + 2];
            reader.read_exact(&mut bytes)?;
            bytes.truncate(length);
            command.push(String::from_utf8(bytes).map_err(std::io::Error::other)?);
        }
        Ok(command)
    }

    fn read_test_line(reader: &mut impl BufRead) -> std::io::Result<String> {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        Ok(line.trim_end_matches(['\r', '\n']).to_owned())
    }
    #[test]
    fn resp_commands_are_binary_safe() {
        let encoded = encode_command(&[
            "GRAPH.QUERY".to_owned(),
            "graphify".to_owned(),
            "RETURN 'a\r\nb'".to_owned(),
        ]);
        assert_eq!(
            encoded,
            b"*3\r\n$11\r\nGRAPH.QUERY\r\n$8\r\ngraphify\r\n$13\r\nRETURN 'a\r\nb'\r\n"
        );
    }

    #[test]
    fn response_parser_handles_nested_arrays_and_reports_errors() -> Result<(), GraphDbError> {
        let mut ok = std::io::Cursor::new(b"*2\r\n+OK\r\n:1\r\n".as_slice());
        let mut budget = 128;
        assert!(matches!(
            read_value(&mut ok, 0, &mut budget)?,
            RespValue::Scalar
        ));

        let mut failure = std::io::Cursor::new(b"-ERR bad query\r\n".as_slice());
        let mut budget = 128;
        assert!(matches!(
            read_value(&mut failure, 0, &mut budget)?,
            RespValue::Error(message) if message == "ERR bad query"
        ));

        let mut blob_failure = std::io::Cursor::new(b"!9\r\nERR blob!\r\n".as_slice());
        let mut budget = 128;
        assert!(matches!(
            read_value(&mut blob_failure, 0, &mut budget)?,
            RespValue::Error(message) if message == "ERR blob!"
        ));

        let mut malformed_bulk = std::io::Cursor::new(b"$3\r\nabcXX".as_slice());
        let mut budget = 128;
        assert!(read_value(&mut malformed_bulk, 0, &mut budget).is_err());
        Ok(())
    }

    #[test]
    fn embedded_credentials_take_precedence_and_are_not_in_endpoint_debug_output()
    -> Result<(), GraphDbError> {
        let endpoint = FalkorEndpoint::parse(
            "falkordb://inside:uri-secret@example.test:6380",
            Some("outside"),
            Some("flag-secret"),
        )?;
        assert_eq!(endpoint.host, "example.test");
        assert_eq!(endpoint.port, 6380);
        assert_eq!(endpoint.user.as_deref(), Some("inside"));
        assert_eq!(endpoint.password.as_deref(), Some("uri-secret"));
        Ok(())
    }
}
