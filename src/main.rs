/*
  clpack init file
*/
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]

#[macro_use]
extern crate lazy_static;

use std::io;
use std::env;

use tokio;
use tokio::net::UdpSocket;
use std::net::SocketAddr;
use std::thread::spawn;

use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::Path;
use std::fs::create_dir_all;

use zstd::{Decoder,Encoder};
use std::num::ParseIntError;

use chrono::prelude::*;
use chrono::Utc;

// version
lazy_static! {
  pub static ref _version: String = getVersion(env!("CARGO_PKG_VERSION"));
}
fn getVersion(version: &str) -> String {
  let mut result: String     = String::new();

  let digits:     Vec<&str>  = version.split('.').collect();
  let digitsSize: usize      = digits.len()-1;
  let mut i:      usize      = 0;

  while i < digitsSize {
    let digit = digits[i];
    if digit != "0" {
      result.push_str(digit);
    }
    if i < digitsSize {
      result.push('.');
    }
    i += 1;
  }
  result
}

// main
mod logger;
#[tokio::main]
async fn main() -> io::Result<()> {
  use crate::logger::*;
  // read args
  let args: Vec<String> = env::args().collect();
  let mut error: bool = true;
  if args.len() > 1 {
    let firstArg = &args[1];
  // no connection part
    // version
    if firstArg == "version" {
      log("bold",&format!("clpack v{}",*_version));
      error = false;
    } else
    // help
    // e:  help
    if firstArg == "help" {
      log("ok","Flags list");
      log("help","┃");
      log("help","┣ version|━━━━━━━━━━━━━━━━╾  Show clpack version");
      log("help","┃");
      log("help","┣ help|━━━━━━━━━━━━━━━━━━━╾  Show avaliable flags");
      log("help","┃");
      log("help","┣ join <server ip>|━━━━━━━╾  Join to server cloud");
      log("help","┃");
      log("help","┣ send <file>|━━━━━━━━━━━━╾  Send file to server");
      log("help","┃");
      log("help","┣ get <file id> <file to>|╾  Get file from server");
      log("help","┃");
      log("help","┗ server|━━━━━━━━━━━━━━━━━╾  Start server cloud");
      error = false;
    } else
    // server
    // e:  server
    if firstArg == "server" {
      log("ok","Start server cloud");
      error = false;
      // files                 name   date   data
      let mut filesList: Vec<(String,String,String)> = getServerFiles("./server")?; // load saved files
      let serverTask = tokio::spawn(async move {
          server(&mut filesList).await.unwrap();
      });
      serverTask.await.unwrap();
  // connection [no joined] part
    } else {
      let mut connection: Option<String> = getConnection(); // get back connection
      // join
      // e:  join 127.0.0.1
      if firstArg == "join" {
        if args.len() == 3 {
          let secondArg = &args[2];
          log("ok",&format!("Join to \"{}\" server and save connection",secondArg));
          error = false;
          setConnection(secondArg.to_string());
          connection = Some(secondArg.to_string());
          //
          client("join").await.unwrap();
        } else {
          log("err","Use the [join <server ip>] flag");
        }
  // connection [joined] part
      } else
      if let Some(connection) = connection {
        // send
        // e:  send test.txt
        if firstArg == "send" {
          if args.len() == 3 {
            let secondArg = &args[2];
            log("ok",&format!("[] Send file \"{}\"",secondArg));
            error = false;
            //
            client(&format!("send {}",secondArg)).await.unwrap();
          } else {
            log("err","Use the [send <file>] flag");
          }
        } else
        // get
        // e:  get 1 test.txt
        if firstArg == "get" {
          if args.len() == 4 {
            let secondArg: &str = &format!("get {} {}",args[2],args[3]);
            log("ok",&format!("[->] \"{}\"",secondArg));
            error = false;
            //
            client(secondArg).await.unwrap();
          } else {
            log("err","Use the [get <file id> <file to>] flag");
          }
        } else
        // list
        // e:  list
        if firstArg == "list" {
          log("ok",&format!("Server: {}",connection));
          log("ok","Files list");
          error = false;
          //
          // todo: 
          //   > server: connections, commands in line
          //   > client: get ip - files
          client("list").await.unwrap();
        }
        //
      } else {
        log("err","No connection");
        logExit();
      }
    }
    //
  }
  if error {
    log("err","Use the [help] flag to show flags list");
    logExit();
  }
  //
  Ok(())
}

const maxPacketSize: usize = 1024;
// send large message
async fn largeRequest(socket: &UdpSocket, addr: &SocketAddr, message: &str) -> std::io::Result<()> {
  if message.is_empty() {
    socket.send_to(b"", addr).await?;
  } else {
    let     bytes: &[u8] = message.as_bytes();
    let mut start: usize = 0;
    let mut end:   usize;

    while start < bytes.len() {
      end = (start+maxPacketSize).min(bytes.len());
      socket.send_to(&bytes[start..end], addr).await?;
      start = end;
    }
  }
  Ok(())
}
// get large message
async fn largeResponse(socket: &UdpSocket) -> io::Result<String> {
  let mut completeMessage: Vec<u8>             = Vec::new();
  let mut buf:             [u8; maxPacketSize] = [0; maxPacketSize];
  loop {
    let (len, _): (usize, SocketAddr) = socket.recv_from(&mut buf).await?;
    completeMessage.extend_from_slice(&buf[..len]);
    if len < buf.len() {
      break;
    }
  }
  // bytes to string
  String::from_utf8(completeMessage)
    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Failed to convert bytes to string"))
}

// server
async fn server(filesList: &mut Vec<(String,String,String)>) -> std::io::Result<()> {
  //
  let addr = "127.0.0.1:8080";
  let socket = UdpSocket::bind(addr).await?;
  println!("Server listening on {}", addr);

  // todo: users list

  // loop
  let mut buf = [0; 1024];
  loop {
    let (len, addr) = socket.recv_from(&mut buf).await?;
    let requestString: String = String::from_utf8_lossy(&buf[..len]).to_string();
    println!("[you] <- [{}] {}",addr,sliceString(&requestString));

    // use command
    let mut response: String = String::new();
    let requestParts: Vec<&str> = requestString.trim_end().split_whitespace().collect();
    match requestParts[0] {
      // join
      "join" => {
      },
      // list
      "list" => {
        for file in &mut *filesList {
          //println!("    - {}",file.0);
          response += &format!("{} {}  ", file.0, file.1);
        }
      },
      // get
      "get" => {
        let fileNum: usize = requestParts[1].parse::<usize>().unwrap();
        if fileNum < filesList.len() { // get num < files list len
          let file = &filesList[fileNum];
          if let Some(hexString) = compressFile( &format!("./server/{} {}",file.0,file.1) ) {
            response += &hexString;
          }
        }
      },
      // send (save in server)
      _ => {
        if requestParts.len() == 3 { // todo: error handler
          let mut data: String = requestParts[2].to_string(); // get first packet
          if requestString.len() >= maxPacketSize {           // get next packets, if > maxPacketSize
            data += &largeResponse(&socket).await?;
          }
          // decompress and save
          if let Some(data) = decompress(&data) {
            bytesToFile(
              &format!(
                "./server/{} {}", // new filename =
                requestParts[0], // name +
                requestParts[1]  // date
              ),
              &data   // data
            );
          }
          // push to server list
          filesList.push( (requestParts[0].to_string(),requestParts[1].to_string(),String::from("0")) );
        }
        //
      }
    }

    // echo the message back to the client
    // todo: status code and client time wait
    largeRequest(&socket,&addr,&response).await?;
  }
}
// client
use std::str::FromStr;
async fn client(arg: &str) -> std::io::Result<()> {
  //
  let addr = SocketAddr::from_str("127.0.0.1:8080")
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
  let socket = UdpSocket::bind("127.0.0.1:0").await?;

  let parts: Vec<&str> = arg.split_whitespace().collect();

  // requests
  // send
  if parts[0] == "send" {
    let filePath = parts[1];
    println!("filePath: {}",filePath);
    if let Some(hexString) = compressFile(filePath) {
      let response:      String = parts[1].to_owned()+" "+&getCurrentTime()+" "+&hexString;
      //let responseBytes: &[u8]  = response.as_bytes();
      largeRequest(&socket,&addr,&response).await?;
      println!("[you -> {}] {}",addr,sliceString(&response));
    } else {
      eprintln!("Error during compression.");
    }
  // other
  } else {
    largeRequest(&socket,&addr,&arg).await?;
    println!("[you -> {}] {}",addr,sliceString(&arg));
  }

  // response
  let responseString: String = largeResponse(&socket).await?;
  println!("[you <- {}] {}",addr,sliceString(&responseString));

  if parts.get(0) == Some(&"list") {
    let leftPart: Vec<&str> = responseString.trim_end().split("  ").collect();
    let maxIndex = leftPart.len()-1;                                     // max index
    let maxIndexWidth = ((maxIndex as f64).log10().floor() as usize) +1; // max index numbers length
    for (i, part) in leftPart.iter().enumerate() {
      println!("  {:>maxIndexWidth$}  {}", i, part.replace(" ", "  "), maxIndexWidth = maxIndexWidth);
    }
  } else 
  if parts[0] == "get" {
    if decompressFile(parts[2], &responseString).is_none() {
      eprintln!("Error during decompression.");
    } else {
      println!("Decompressed content written to \"{}\"",parts[2]);
    }
  }

  //
  Ok(())
}

// save ip:port to file
fn setConnection(server: String) -> Option<()> {
  let path = "connection.txt";
  if let Err(_) = OpenOptions::new()
    .write(true)
    .create(true)
    .truncate(true) // delete back
    .open(path)
    .and_then(|mut file| file.write_all(server.as_bytes()))
  {
    return None;
  }
  Some(())
}
// get ip:port string from file
fn getConnection() -> Option<String> {
  let path = "connection.txt";
  let mut file = match File::open(path) {
    Ok(file) => file,
    Err(_) => return None,
  };
  
  let mut server = String::new();
  if file.read_to_string(&mut server).is_err() {
    return None;
  }
  
  Some(server)
}

// get current time
fn getCurrentTime() -> String {
  // Получаем текущее время в UTC
  let now = Utc::now();
  // Форматируем время в строку "день-месяц-год-мс-сек-минута-час"
  format!("{:02}-{:02}-{}-{:03}-{:02}-{:02}-{:02}",
    now.day(), now.month(), now.year(),
    now.timestamp_subsec_millis(), now.second(), now.minute(), now.hour()
  )
}

// compress file
// use base64;
// todo: compress bytes and send bytes, not String !!!
// todo: use zstd + base64 + word crypt
fn compressFile(filePath: &str) -> Option<String> {
    let mut file = File::open(filePath).ok()?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).ok()?;

    let hexString = buffer.iter().map(|b| format!("{:02x}", b)).collect::<String>();
    Some(hexString)
}
fn decompressFile(outputPath: &str, hexString: &str) -> Option<()> {
    let bytes = (0..hexString.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hexString[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    
    let mut file = File::create(outputPath).ok()?;
    file.write_all(&bytes).ok()?;
    Some(())
}
// save bytes to file, and if needed create directory
fn bytesToFile(fileName: &str, content: &Vec<u8>) -> Option<()> {
  let path = Path::new(fileName);

  // directory
  if let Some(parent) = path.parent() {
    // create if exists
    if create_dir_all(parent).is_err() {
      return None;
    }
  }

  // create file
  let mut file = match File::create(fileName) {
    Ok(f) => f,
    Err(_) => return None,
  };

  // write data to file
  if let Err(_) = file.write_all(content) {
    return None;
  }

  Some(())
}
// decompress str to bytes
fn decompress(hexString: &str) -> Option<Vec<u8>> {
    let bytes: Result<Vec<u8>, ParseIntError> = (0..hexString.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hexString[i..i+2], 16))
        .collect();

    match bytes {
        Ok(b) => Some(b),
        Err(_) => None,
    }
}

// slice of string
fn sliceString(input: &str) -> String {
    let mut space_count = 0;
    let mut truncateIndex = None;
    
    for (index, char) in input.char_indices() {
        if char == ' ' {
            space_count += 1;
            if space_count == 3 {
                truncateIndex = Some(index);
                break;
            }
        }
    }

    // if spaces
    if let Some(index) = truncateIndex {
        let truncated = &input[..index];
        let byte_count = truncated.len();
        return format!("{}<{} bytes>", truncated, byte_count);
    }
    // if bytest
    if input.len() > 32 {
        let truncated = &input[..32];
        let byte_count = truncated.len();
        return format!("<{} bytes>", byte_count);
    // if basic
    } else {
        return format!("{}", input);
    }
}


// get save server files
// todo: check directory exists
fn getServerFiles(path: &str) -> io::Result<Vec<(String,String,String)>> {
    let mut result = Vec::new();
    let entries    = fs::read_dir(path)?; // get dir files

    // read files
    for entry in entries {
      let entry = entry?;
      let path  = entry.path();

      // if file then get name-data
      if path.is_file() {
        let fileName: String = path.file_name()
          .and_then(|name| name.to_str())
          .unwrap_or("")
          .to_string();

        let parts: Vec<&str> = fileName.split(" ").collect();
        let content = fs::read_to_string(&path).unwrap_or_else(|_| String::from("Failed to read file"));

        result.push((parts[0].to_string(),parts[1].to_string(),content));
      // todo: error
      } else {
        println!("Skipping non-file entry: {:?}", path.display());
      }
    }

    Ok(result)
}