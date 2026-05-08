use flume::{unbounded, RecvError, TryRecvError};
use log::{error, info, warn};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    thread,
    time::{Duration, SystemTime},
};
use tokio::{runtime::Runtime, select, task, time::sleep};
use zenoh::query::{Parameters, Selector};
use zenoh::{
    bytes::{Encoding, ZBytes},
    handlers::FifoChannelHandler,
    key_expr::OwnedKeyExpr,
    liveliness::LivelinessToken,
    pubsub::Subscriber,
    qos::{CongestionControl, Priority},
    query::{QueryConsolidation, QueryTarget, Reply},
    sample::{Locality, Sample},
    Config, Session,
};

pub type Sender<T> = flume::Sender<T>;
pub type Receiver<T> = flume::Receiver<T>;

pub struct SessionInfoData {
    pub zid: String,
    pub peers: Vec<String>,
    pub routers: Vec<String>,
}

pub struct SubData {
    pub id: u64,
    pub key_expr: OwnedKeyExpr,
    pub origin: Locality,
}

pub struct PutData {
    pub id: u64,
    pub key: OwnedKeyExpr,
    pub congestion_control: CongestionControl,
    pub priority: Priority,
    pub encoding: Encoding,
    pub payload: ZBytes,
}

pub struct QueryData {
    pub id: u64,
    pub key_expr: OwnedKeyExpr,
    pub parameters: Parameters<'static>,
    pub attachment: Option<ZBytes>,
    pub target: QueryTarget,
    pub consolidation: QueryConsolidation,
    pub locality: Locality,
    pub timeout: Duration,
    pub value: Option<(Encoding, ZBytes)>,
}

pub struct LivelinessTokenData {
    pub id: u64,
    pub key_expr: OwnedKeyExpr,
}

pub struct LivelinessSubData {
    pub id: u64,
    pub key_expr: OwnedKeyExpr,
}

pub struct AdminQueryData {
    pub id: u64,
    pub key_expr: String,
    pub timeout_ms: u64,
}

pub enum MsgGuiToZenoh {
    Close,
    AddSubReq(Box<SubData>),
    DelSubReq(u64),
    GetReq(Box<QueryData>),
    PutReq(Box<PutData>),
    GetSessionInfo,
    DeclareLivelinessToken(Box<LivelinessTokenData>),
    UndeclareLivelinessToken(u64),
    AddLivelinessSubReq(Box<LivelinessSubData>),
    DelLivelinessSubReq(u64),
    AdminQueryReq(Box<AdminQueryData>),
}

pub enum MsgZenohToGui {
    OpenSession(Result<u64, (u64, String)>),
    AddSubRes(Box<(u64, Result<(), String>)>),
    DelSubRes(u64),
    SubCB(Box<(u64, Sample, SystemTime)>),
    GetRes(Box<(u64, Reply)>),
    PutRes(Box<(u64, bool, String)>),
    SessionInfo(Box<SessionInfoData>),
    DeclareLivelinessTokenRes(Box<(u64, Result<(), String>)>),
    AddLivelinessSubRes(Box<(u64, Result<(), String>)>),
    DelLivelinessSubRes(u64),
    LivelinessSubCB(Box<(u64, Sample, SystemTime)>),
    AdminQueryRes(Box<(u64, String, String, Vec<u8>, bool)>), // (query_id, key, encoding, payload, is_ok)
    AdminQueryDone(u64),
}

pub fn start_async(
    sender_to_gui: Sender<MsgZenohToGui>,
    receiver_from_gui: Receiver<MsgGuiToZenoh>,
    id: u64,
    config_file_path: PathBuf,
) {
    thread::spawn(move || {
        let rt = Runtime::new().unwrap();
        rt.block_on(loop_zenoh(
            sender_to_gui,
            receiver_from_gui,
            config_file_path,
            id,
        ));
    });
}

async fn loop_zenoh(
    sender_to_gui: Sender<MsgZenohToGui>,
    receiver_from_gui: Receiver<MsgGuiToZenoh>,
    config_file_path: PathBuf,
    id: u64,
) {
    let config = match Config::from_file(config_file_path) {
        Ok(o) => o,
        Err(e) => {
            let s = e.to_string();
            warn!("{s}");
            let _ = sender_to_gui.send(MsgZenohToGui::OpenSession(Err((id, s))));
            return;
        }
    };

    let session: Session = match zenoh::open(config).await {
        Ok(o) => {
            info!("session open ok");
            o
        }
        Err(e) => {
            let s = e.to_string();
            warn!("{s}");
            let _ = sender_to_gui.send(MsgZenohToGui::OpenSession(Err((id, s))));
            return;
        }
    };
    let _ = sender_to_gui.send(MsgZenohToGui::OpenSession(Ok(id)));

    let mut subscriber_senders: BTreeMap<u64, Sender<()>> = BTreeMap::new();
    let mut liveliness_tokens: BTreeMap<u64, LivelinessToken> = BTreeMap::new();
    let mut liveliness_senders: BTreeMap<u64, Sender<()>> = BTreeMap::new();

    'a: loop {
        let try_read = receiver_from_gui.try_recv();
        let msg = match try_read {
            Ok(m) => m,
            Err(e) => {
                match e {
                    TryRecvError::Empty => {
                        sleep(Duration::from_millis(8)).await;
                        continue 'a;
                    }
                    TryRecvError::Disconnected => {
                        break 'a;
                    }
                };
            }
        };
        match msg {
            MsgGuiToZenoh::Close => {
                break 'a;
            }
            MsgGuiToZenoh::AddSubReq(req) => {
                let SubData {
                    id,
                    key_expr,
                    origin,
                } = *req;
                let subscriber: Subscriber<FifoChannelHandler<Sample>> = match session
                    .declare_subscriber(key_expr)
                    .allowed_origin(origin)
                    .await
                {
                    Ok(o) => o,
                    Err(e) => {
                        let _ = sender_to_gui
                            .send(MsgZenohToGui::AddSubRes(Box::new((id, Err(e.to_string())))));
                        return;
                    }
                };
                let (close_sender, close_receiver): (Sender<()>, Receiver<()>) = unbounded();
                let _ = subscriber_senders.insert(id, close_sender);
                task::spawn(task_subscriber(
                    id,
                    subscriber,
                    close_receiver,
                    sender_to_gui.clone(),
                ));
                let _ = sender_to_gui.send(MsgZenohToGui::AddSubRes(Box::new((id, Ok(())))));
            }
            MsgGuiToZenoh::DelSubReq(id) => {
                if let Some(sender) = subscriber_senders.get(&id) {
                    let _ = sender.send(());
                }
                let _ = subscriber_senders.remove(&id);
                let _ = sender_to_gui.send(MsgZenohToGui::DelSubRes(id));
            }
            MsgGuiToZenoh::GetReq(req) => {
                task::spawn(task_query(session.clone(), req, sender_to_gui.clone()));
            }
            MsgGuiToZenoh::GetSessionInfo => {
                task::spawn(task_session_info(session.clone(), sender_to_gui.clone()));
            }
            MsgGuiToZenoh::DeclareLivelinessToken(data) => {
                let LivelinessTokenData { id, key_expr } = *data;
                match session.liveliness().declare_token(key_expr).await {
                    Ok(token) => {
                        liveliness_tokens.insert(id, token);
                        let _ = sender_to_gui.send(MsgZenohToGui::DeclareLivelinessTokenRes(
                            Box::new((id, Ok(()))),
                        ));
                    }
                    Err(e) => {
                        let _ = sender_to_gui.send(MsgZenohToGui::DeclareLivelinessTokenRes(
                            Box::new((id, Err(e.to_string()))),
                        ));
                    }
                }
            }
            MsgGuiToZenoh::UndeclareLivelinessToken(id) => {
                liveliness_tokens.remove(&id);
            }
            MsgGuiToZenoh::AddLivelinessSubReq(data) => {
                let LivelinessSubData { id, key_expr } = *data;
                match session.liveliness().declare_subscriber(key_expr).await {
                    Ok(subscriber) => {
                        let (close_sender, close_receiver) = unbounded();
                        liveliness_senders.insert(id, close_sender);
                        task::spawn(task_liveliness_subscriber(
                            id,
                            subscriber,
                            close_receiver,
                            sender_to_gui.clone(),
                        ));
                        let _ = sender_to_gui.send(MsgZenohToGui::AddLivelinessSubRes(Box::new(
                            (id, Ok(())),
                        )));
                    }
                    Err(e) => {
                        let _ = sender_to_gui.send(MsgZenohToGui::AddLivelinessSubRes(Box::new(
                            (id, Err(e.to_string())),
                        )));
                    }
                }
            }
            MsgGuiToZenoh::DelLivelinessSubReq(id) => {
                if let Some(sender) = liveliness_senders.remove(&id) {
                    let _ = sender.send(());
                }
                let _ = sender_to_gui.send(MsgZenohToGui::DelLivelinessSubRes(id));
            }
            MsgGuiToZenoh::AdminQueryReq(data) => {
                task::spawn(task_admin_query(session.clone(), data, sender_to_gui.clone()));
            }
            MsgGuiToZenoh::PutReq(p) => {
                let pd = *p;
                if let Err(e) = session
                    .put(pd.key.clone(), pd.payload)
                    .encoding(pd.encoding)
                    .congestion_control(pd.congestion_control)
                    .priority(pd.priority)
                    .await
                {
                    let s = format!("put error \"{}\", {}", pd.key, e);
                    warn!("{s}");
                    let _ = sender_to_gui.send(MsgZenohToGui::PutRes(Box::new((pd.id, false, s))));
                } else {
                    let s = format!("put ok \"{}\"", pd.key);
                    info!("{s}");
                    let _ = sender_to_gui.send(MsgZenohToGui::PutRes(Box::new((pd.id, true, s))));
                }
            }
        }
    }

    for (sub_id, sender) in subscriber_senders {
        let _ = sender.send(());
        let _ = sender_to_gui.send(MsgZenohToGui::DelSubRes(sub_id));
    }

    drop(liveliness_tokens);

    for (id, sender) in liveliness_senders {
        let _ = sender.send(());
        let _ = sender_to_gui.send(MsgZenohToGui::DelLivelinessSubRes(id));
    }

    info!("session closed");
}

async fn task_subscriber(
    id: u64,
    subscriber: Subscriber<FifoChannelHandler<Sample>>,
    close_receiver: Receiver<()>,
    sender_to_gui: Sender<MsgZenohToGui>,
) {
    info!("task_subscriber entry");
    'a: loop {
        let r: Result<Sample, RecvError> = select!(
            sample = subscriber.recv_async() =>{
                 sample.map_err(|_|RecvError::Disconnected)
            },

            _ = close_receiver.recv_async() =>{
                 Err(RecvError::Disconnected)
            },
        );

        match r {
            Ok(sample) => {
                let t = SystemTime::now();
                let msg = MsgZenohToGui::SubCB(Box::new((id, sample, t)));
                if let Err(e) = sender_to_gui.send(msg) {
                    println!("{}", e);
                }
            }
            Err(_) => {
                break 'a;
            }
        }
    }
    info!("task_subscriber exit");
}

async fn task_session_info(session: Session, sender: Sender<MsgZenohToGui>) {
    let info = session.info();
    let zid = info.zid().await.to_string();
    let peers = info
        .peers_zid()
        .await
        .into_iter()
        .map(|id| id.to_string())
        .collect();
    let routers = info
        .routers_zid()
        .await
        .into_iter()
        .map(|id| id.to_string())
        .collect();
    let _ = sender.send(MsgZenohToGui::SessionInfo(Box::new(SessionInfoData {
        zid,
        peers,
        routers,
    })));
}

async fn task_liveliness_subscriber(
    id: u64,
    subscriber: Subscriber<FifoChannelHandler<Sample>>,
    close_receiver: Receiver<()>,
    sender_to_gui: Sender<MsgZenohToGui>,
) {
    'a: loop {
        let r: Result<Sample, RecvError> = select!(
            sample = subscriber.recv_async() => {
                sample.map_err(|_| RecvError::Disconnected)
            },
            _ = close_receiver.recv_async() => {
                Err(RecvError::Disconnected)
            },
        );
        match r {
            Ok(sample) => {
                let t = SystemTime::now();
                let _ = sender_to_gui.send(MsgZenohToGui::LivelinessSubCB(Box::new((id, sample, t))));
            }
            Err(_) => break 'a,
        }
    }
}

async fn task_admin_query(
    session: Session,
    data: Box<AdminQueryData>,
    sender_to_gui: Sender<MsgZenohToGui>,
) {
    let id = data.id;
    let timeout = Duration::from_millis(data.timeout_ms);
    info!("task_admin_query entry, key \"{}\"", data.key_expr);

    let replies = match session.get(data.key_expr.as_str()).timeout(timeout).await {
        Ok(r) => r,
        Err(e) => {
            warn!("admin query error: {}", e);
            let _ = sender_to_gui.send(MsgZenohToGui::AdminQueryDone(id));
            return;
        }
    };

    while let Ok(reply) = replies.recv_async().await {
        let (key, encoding, payload, is_ok) = match reply.result() {
            Ok(sample) => {
                let key = sample.key_expr().to_string();
                let encoding = sample.encoding().to_string();
                let payload = sample.payload().to_bytes().to_vec();
                (key, encoding, payload, true)
            }
            Err(err) => {
                let encoding = err.encoding().to_string();
                let payload = err.payload().to_bytes().to_vec();
                (String::new(), encoding, payload, false)
            }
        };
        let _ = sender_to_gui.send(MsgZenohToGui::AdminQueryRes(Box::new((id, key, encoding, payload, is_ok))));
    }

    let _ = sender_to_gui.send(MsgZenohToGui::AdminQueryDone(id));
    info!("task_admin_query exit");
}

async fn task_query(session: Session, data: Box<QueryData>, sender_to_gui: Sender<MsgZenohToGui>) {
    let d = *data;
    let key_expr_str = d.key_expr.to_string();
    info!("task_query entry, key expr \"{}\"", key_expr_str);

    let selector = Selector::from((d.key_expr, d.parameters));
    let replies = match d.value {
        Some((encoding, payload)) => session.get(selector).payload(payload).encoding(encoding),
        None => session.get(selector),
    }
    .attachment(d.attachment)
    .target(d.target)
    .timeout(d.timeout)
    .consolidation(d.consolidation)
    .allowed_destination(d.locality)
    .await
    .unwrap();

    while let Ok(reply) = replies.recv_async().await {
        let msg = MsgZenohToGui::GetRes(Box::new((d.id, reply)));
        if let Err(e) = sender_to_gui.send(msg) {
            error!("{}", e);
        }
    }

    info!("task_query exit, key expr {}", key_expr_str);
}
