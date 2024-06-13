use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use log::{error, info, LevelFilter};
use russh::server::{Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use russh_keys::key::{self, KeyPair};
use russh_sftp::protocol::{Attrs, File, FileAttributes, Handle, Name, OpenFlags, Packet, Status, StatusCode, Version};
use sea_orm::{Database, DatabaseConnection, EntityTrait, QueryFilter};
use tokio::sync::Mutex;
use sea_orm::entity::ColumnTrait;

use myent::sftp::Entity as SftpEntity;

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
        let mut clients = self.clients.lock().await;
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
                println!("pubkey: {:?}", pubkey);

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
            let mut clients = self.clients.lock().await;
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
            let sftp = SftpSession::new(self.mountpoint.clone().unwrap());
            
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
}

impl SftpSession {
    pub fn new(mountpoint: String) -> Self {
        Self {
            version: None,
            mountpoint: Some(mountpoint),
            root_dir_read_done: false,
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

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        Err(self.unimplemented())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        Err(self.unimplemented())
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        Err(self.unimplemented())
    }

    async fn extended(
        &mut self,
        id: u32,
        request: String,
        data: Vec<u8>,
    ) -> Result<Packet, Self::Error> {
        Err(self.unimplemented())
    }

    async fn symlink(
        &mut self,
        id: u32,
        linkpath: String,
        targetpath: String,
    ) -> Result<Status, Self::Error> {
        Err(self.unimplemented())
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        Err(self.unimplemented())
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        Err(self.unimplemented())
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        Err(self.unimplemented())
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        Err(self.unimplemented())
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        Err(self.unimplemented())
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        Err(self.unimplemented())
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        Err(self.unimplemented())
    }

    async fn close(&mut self, id: u32, _handle: String) -> Result<Status, Self::Error> {
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
        Err(self.unimplemented())
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        info!("readdir handle: {}", handle);
        if handle == "/" && !self.root_dir_read_done {
            self.root_dir_read_done = true;
            return Ok(Name {
                id,
                files: vec![
                    File {
                        filename: "foo".to_string(),
                        longname: "".to_string(),
                        attrs: FileAttributes::default(),
                    },
                    File {
                        filename: "bar".to_string(),
                        longname: "".to_string(),
                        attrs: FileAttributes::default(),
                    },
                ],
            });
        }
        Ok(Name { id, files: vec![] })
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        info!("realpath: {}", path);
        Ok(Name {
            id,
            files: vec![File {
                filename: "/".to_string(),
                longname: "".to_string(),
                attrs: FileAttributes::default(),
            }],
        })
    }
}

#[tokio::main]
async fn main() {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        .init();
    

    let config = russh::server::Config {
        auth_rejection_time: Duration::from_secs(3),
        auth_rejection_time_initial: Some(Duration::from_secs(0)),
        keys: vec![KeyPair::generate_ed25519().unwrap()],
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