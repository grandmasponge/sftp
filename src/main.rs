use std::collections::HashMap;
use std::fmt::format;
use std::io::{Read, Write};
use std::{fs, path};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use log::{error, info, LevelFilter};
use russh::server::{Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use russh_keys::key::{self, KeyPair};
use russh_keys::PublicKeyBase64;
use russh_sftp::protocol::{Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Packet, Status, StatusCode, Version};
use sea_orm::{Database, DatabaseConnection, EntityTrait, QueryFilter};
use sea_orm::entity::ColumnTrait;

use myent::sftp::Entity as SftpEntity;
use tokio::io::{AsyncReadExt, AsyncWriteExt};



#[derive(Clone)]
struct Server;

impl russh::server::Server for Server {
    type Handler = SshSession;

    fn new_client(&mut self, _: Option<SocketAddr>) -> Self::Handler {
        println!("new client");
        SshSession::default()
    }
}

struct SshSession {
    clients: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
    mountpoint: Option<String>,
    db: Option<Arc<DatabaseConnection>>,
}

impl Default for SshSession {
    fn default() -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            mountpoint: None,
            db: None,
        }
    }
}

impl SshSession {
    pub async fn get_channel(&mut self, channel_id: ChannelId) -> Channel<Msg> {
        let mut clients = self.clients.lock().unwrap();
        clients.remove(&channel_id).unwrap()
    }
}

#[async_trait]
impl russh::server::Handler for SshSession {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {

        let db = Database::connect("mysql://root@localhost:3306/sftp").await?;
        info!("connected to db");

        self.db = Some(Arc::new(db.clone()));

        let user = SftpEntity::find()
            .filter(myent::sftp::Column::Username.eq(user))
            .one(&db)
            .await?;
        let user = match user {
            Some(user) => user,
            None => {
                info!("user not found");
                return Ok(Auth::Reject { proceed_with_methods: None });
            }
        };

        if let Some(upass) = user.password {
            if upass == password {
                info!("password match");
                self.mountpoint = Some(user.mountpoint);
                return Ok(Auth::Accept);
            }
            else {
                info!("password mismatch");
                return Ok(Auth::Reject { proceed_with_methods: None });
            }
        }

        info!("password not set");
        Ok(Auth::Reject { proceed_with_methods: None })
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        info!("auth_publickey: {} with key {:?}", user, public_key);

        let db = Database::connect("mysql://root@localhost:3306/sftp")
            .await?;
        info!("connected to db");

        self.db = Some(Arc::new(db.clone()));

        let user = SftpEntity::find()
        .filter(myent::sftp::Column::Username.eq(user))
        .one(&db)
        .await?;

        let user = match user {
            Some(user) => user,
            None => {
                info!("user not found");
                return Ok(Auth::Reject { proceed_with_methods: None });
            }
        };

        let pubkey = user.keys;

        match pubkey {
            Some(pubkey) => {
                info!("pubkey: {:?}", pubkey.as_object());
                

                return Ok(Auth::Accept)
            },
            None => {
                info!("public key not set");
                return Ok(Auth::Reject { proceed_with_methods: None });
            }
        }

    }
   
   
    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        {
            let mut clients = self.clients.lock().unwrap();
            clients.insert(channel.id(), channel);
        }
        Ok(true)
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        info!("subsystem: {}", name);
        
        if name == "sftp" {
            let channel = self.get_channel(channel_id).await;
            let sftp = SftpSession::new("mountpoints/u1".to_string());
            
            session.channel_success(channel_id);
            russh_sftp::server::run(channel.into_stream(), sftp).await;
        } else {
            session.channel_failure(channel_id);
        }

        Ok(())
    }
}

#[derive(Default)]
struct SftpSession {
    version: Option<u32>,
    mountpoint: Option<String>,
    root_dir_read_done: bool,
    handles: Arc<Mutex<HashMap<String, fs::File>>>
}

impl SftpSession {
    pub fn new(mountpoint: String) -> Self {
        Self {
            version: None,
            mountpoint: Some(mountpoint),
            root_dir_read_done: false,
            handles: Arc::new(Mutex::new(HashMap::new()))
        }
    }
}

#[async_trait]
impl russh_sftp::server::Handler for SftpSession {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        version: u32,
        extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        if self.version.is_some() {
            error!("duplicate SSH_FXP_VERSION packet");
            return Err(StatusCode::ConnectionLost);
        }
        

        self.version = Some(version);
        info!("version: {:?}, extensions: {:?}", self.version, extensions);
        Ok(Version::new())
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        info!("read: {}", handle);
        println!("len: {len}");
        println!("off: {offset}");
        let mount = self.mountpoint
        .clone()
        .unwrap();
        let path = format!("./{}{}",mount,handle);
            let mut buf: [u8; 1024] = [0;1024];
            let mut file = tokio::fs::File::open(path)
            .await
            .unwrap();
            let res = file.read(&mut buf).await.unwrap();

            println!("{res}");
            println!("{buf:?}");
           return  Ok(Data { id, data: buf.to_vec() });

    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {

        let mountpoint = self.mountpoint.clone()
        .unwrap();
        let path = format!("./{}{}", mountpoint, filename);
        let _ = tokio::fs::remove_file(path)
        .await
        .unwrap();

        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-UK".to_string(),
        })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        info!("write: {}", handle);
        info!("offset: {}", offset);
        info!("data:{:?}", data);

        let mount = self.mountpoint.clone().unwrap();
        let path = format!("./{}{}", mount, handle);

        //test
        let mut file = tokio::fs::File::create(path)
        .await
        .unwrap();
       
        //test over
        let res = file.write_all(&data)
        .await
        .unwrap();
        
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-UK".to_string(),
        })
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let mountpoint = self.mountpoint.clone().unwrap();
        let path = format!("{}{}", mountpoint, filename);
        info!("open: {}", path);
        let mut file = std::fs::OpenOptions::from(pflags)
        .open("./mountpoints/u1/haha/tehe.txt")
        .unwrap();

        let mut handles = self.handles.lock()
        .unwrap();
        handles.insert(filename.clone(), file);

        drop(handles);
        
        let handle = filename.clone();
        let wha: u32 = 0345;

        info!("handle: {}", handle);
        Ok(Handle { id, handle })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let mountpoint = self.mountpoint.clone().unwrap();
        let path = format!("./{}{}", mountpoint, path);
        info!("stat: {}", path);
        let path = Path::new(path.as_str());
        let metadata = match path.metadata() {
            Ok(data) => {
                info!("is dir");
                data
            },
            Err(_e) => {
                info!("is file");
                let file = std::fs::File::open("./mountpoints/u1/haha/tehe.txt")
                .unwrap();

                let data = file.metadata()
                .unwrap();
                data
            }
        };
        let attrs = FileAttributes::from(&metadata);

        Ok(Attrs { id, attrs })
    }

    async fn extended(
        &mut self,
        id: u32,
        request: String,
        data: Vec<u8>,
    ) -> Result<Packet, Self::Error> {
        println!("what ever this is");
        Err(self.unimplemented())
    }

   

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let mountpoint = self.mountpoint.clone().unwrap();
        let path = format!("./{}{}", mountpoint, path);
        info!("stat: {}", path);
        let path = Path::new(path.as_str());
        let metadata = match path.metadata() {
            Ok(data) => {
                info!("is dir");
                data
            },
            Err(_e) => {
                info!("is file");
                let file = std::fs::File::open("./mountpoints/u1/haha/tehe.txt")
                .unwrap();

                let data = file.metadata()
                .unwrap();
                data
            }
        };
        let attrs = FileAttributes::from(&metadata);

        Ok(Attrs { id, attrs })
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        info!("rename: {} to {}", oldpath, newpath);
        let mountpoint = self.mountpoint.clone().unwrap();
        let oldpath = format!("./{}/{}", mountpoint, oldpath);
        let newpath = format!("./{}/{}", mountpoint, newpath);
        std::fs::rename(oldpath, newpath).unwrap();

        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-UK".to_string(),
        })

    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        let mountpoint = self.mountpoint.clone().unwrap();
        let path = format!("./{}/{}", mountpoint, path);
        info!("mkdir: {}", path);
        std::fs::create_dir(path).unwrap();

        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-UK".to_string(),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        info!("fstat: {}", handle);

        let mountpoint = self.mountpoint.clone().unwrap();
        let path = format!("./{}{}", mountpoint, handle);

        let file = std::fs::File::open(path)
        .unwrap();

        let metadata = file
        .metadata()
        .unwrap();

        let attrs = FileAttributes::from(&metadata);

        Ok(Attrs {
            id,
            attrs
        })
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        info!("setstat: {}", path);
        Err(self.unimplemented())
    }



    async fn close(&mut self, id: u32, _handle: String) -> Result<Status, Self::Error> {
        info!("closing file");
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-UK".to_string(),
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        info!("opendir: {}", path);
        self.root_dir_read_done = false;
       
        Ok(Handle { id, handle: path })
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        let mountpoint = self.mountpoint.clone().unwrap();
        let path = format!("./{}/{}", mountpoint, path);
        info!("rmdir: {}", path);
        std::fs::remove_dir(path).unwrap();
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-UK".to_string(),
        })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
       info!("readdir handle: {}", handle);
        if !self.root_dir_read_done {
            let mountpoint = self.mountpoint.clone().unwrap();
            let path = format!("./{}/{}", mountpoint, handle);
            info!("readdir: {}", path);
            let path = Path::new(path.as_str());
            let mut files = vec![];

            for entry in path.read_dir().unwrap() {
                let entry = entry.unwrap();
                let filename = entry.file_name().to_str().unwrap().to_string();
                let longname = filename.clone();
                let metadata = entry.metadata().unwrap();
                let mut attrs = FileAttributes::from(&metadata);
                let ftype = entry.file_type().unwrap();
                
                if ftype.is_dir() {
                    attrs.set_dir(true);
                } else if ftype.is_file() {
                    attrs.set_regular(true);
                } else if ftype.is_symlink() {
                    attrs.set_symlink(true);
                }
            
                files.push(File {
                    filename,
                    longname,
                    attrs,
                });
            }
            self.root_dir_read_done = true;
            Ok(Name { id, files })

        } else {
            Err(StatusCode::Eof)
        }

    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        info!("realpath: {}", path);
    
        let mut path = match path.as_str() {
            "." => "/".to_string(),
            "" => "/".to_string(),
            _ => path,
        };
        
        if path.is_empty() {
            path = "/".to_string();
        }
    
        info!("path: {}", path);
    
        while path.contains("..") {
            let parts: Vec<&str> = path.split('/').collect();
            let mut new_parts: Vec<&str> = Vec::with_capacity(parts.len());
    
            for part in parts {
                match part {
                    ".." => {
                        if !new_parts.is_empty() {
                            new_parts.pop();
                        }
                    }
                    "." => {}
                    _ => {
                        if !(part.is_empty() && new_parts.is_empty()) {
                            new_parts.push(part);
                        }
                    }
                }
            }
    
            if new_parts.is_empty() {
                path = "/".to_string();
            } else {
                path = new_parts.join("/");
            }
        }
    
        if path.is_empty() {
            path = "/".to_string();
        }
    
        Ok(Name {
            id,
            files: vec![File {
                filename: path.clone(),
                longname: "".to_string(),
                attrs: FileAttributes::default(),
            }],
        })
    }
        

   
}

fn loadpubkey(path: String) -> KeyPair {
    let private = russh_keys::load_secret_key(path, None)
    .unwrap();

    private
}

#[tokio::main]
async fn main() {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        .init();

    let keypath = "./keys/id_ed".to_string();
    let key = loadpubkey(keypath);
    

    let config = russh::server::Config {
        auth_rejection_time: Duration::from_secs(3),
        auth_rejection_time_initial: Some(Duration::from_secs(0)),
        keys: vec![key],
        ..Default::default()
    };

    let mut server = Server;

    server
        .run_on_address(
            Arc::new(config),
            (
                "127.0.0.1",
                std::env::var("PORT")
                    .unwrap_or("3000".to_string())
                    .parse()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
}