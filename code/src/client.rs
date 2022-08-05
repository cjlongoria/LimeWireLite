use tonic::{transport::Server, Request, Response, Status};
use p2p_ft::greeter_client::GreeterClient;
use p2p_ft::ftp_client::FtpClient;
// ***************Start Change***************
use p2p_ft::{ListRequest, RegisterRequest, DeregisterRequest, 
    ReceiveRequest, ReceiveReply, GenericReply, 
    QueryResponse, QueryReply, QueryBroadcast, 
    QueryInitial, UpdateRequest, CheckReply, 
    DeleteRequest, FileStatusReply, StateUpdate};
    // ***************End Change***************
use p2p_ft::ftp_server::{Ftp, FtpServer};
use p2p_ft::query_engine_server::{QueryEngine, QueryEngineServer};
use p2p_ft::query_engine_client::QueryEngineClient;
use std::io::{self, Write};
use std::{fs,env};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use mac_address::get_mac_address;
use std::thread;
use std::sync::mpsc;
use std::sync::Mutex;
use tokio::sync::Mutex as AMutex;
use local_ip_address::local_ip;
use std::time::Duration;
use rand::Rng;
use configparser::ini::Ini;
use std::path::Path;
use std::time::SystemTime;

// This is used to pull in the RPC services/parameters
pub mod p2p_ft {
    tonic::include_proto!("p2pft");
}

///////////////////////////////////////////////////////////////////////////////////////////////////////////
/// FTP Server
// This sets up the functions for the file transfer service between clients

#[derive(Debug, Default)]
pub struct MyFTP {
    // filename, ip, ttr, expired_time, state
    filelist: Mutex<Vec<(String, String, u64, u64, u64)>>,
    filelist_expired: Mutex<Vec<(String, String)>>,
    ip: String,
}

//The client sends its info (IP, port, requested filename) to a client with the file it wants then waits to recieve the file.
//This is done using a threadpool so that it can receive multiple files at the same time.
#[tonic::async_trait]
impl Ftp for MyFTP {

    // ***************Start Change***************
    async fn receive(
        &self,
        request: Request<ReceiveRequest>,
    ) -> Result<Response<ReceiveReply>, Status> {

        let filename = request.into_inner().filename;
        
        let mut file = self.filelist.lock().unwrap().clone();
        file.retain(|(filename_local,_, _, _, _)| *filename_local == filename);

        let (filename, ip, ttr, expire_time, state) = file.pop().unwrap();

        if ip.eq("") {
            let path_name = format!("{}/Owned/{}", env::current_dir().unwrap().display().to_string(), filename);

            let file = fs::read(path_name).unwrap();
            let reply = p2p_ft::ReceiveReply {
                file_bytes: file,
                ip: self.ip.clone(),
                ttr,
                expire_time,
                state,
            };
            Ok(Response::new(reply))
        }
        else{
            let path_name = format!("{}/Downloaded/{}", env::current_dir().unwrap().display().to_string(), filename);

            let file = fs::read(path_name).unwrap();
            let reply = p2p_ft::ReceiveReply {
                file_bytes: file,
                ip,
                ttr,
                expire_time,
                state,
            };
            Ok(Response::new(reply))
        }
        
    }
    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<GenericReply>, Status> {

        let filename = request.get_ref().filename.clone();
        let ip = request.get_ref().ip.clone();
        let ttr = request.get_ref().ttr;
        let expire_time = request.get_ref().expire_time;
        let state = request.get_ref().state;

        let mut flist = self.filelist.lock().unwrap();
        flist.push((filename,ip,ttr,expire_time,state));
        // Debug to print the full filelist that the FTP server has
        // println!("{:?}",flist);


        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    async fn refresh(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<GenericReply>, Status> {
        
        let flist_expired = {
            let flist_expired = self.filelist_expired.lock().unwrap();
            flist_expired.clone()
        };
        
        //(filename, ip)
        for file_tuple in flist_expired{
            let filename = file_tuple.0;
            let ip = file_tuple.1;

            let client_addr = format!("{}", ip.clone());
            let mut client = FtpClient::connect(client_addr.clone()).await.unwrap();
            
            let path_name = format!("{}/Downloaded/{}", env::current_dir()
                .unwrap()
                .display()
                .to_string(), &filename);
            

            let request = tonic::Request::new(ReceiveRequest {
                filename: filename.clone(),
            });

            let response = client.receive(request).await?;
            let ttr = response.get_ref().ttr.clone();
            let expire_time = response.get_ref().expire_time.clone();
            let state = response.get_ref().state.clone();

            let expire_time = { match expire_time {
                0 => SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() + ttr,
                _ => expire_time,
                }
            };
            fs::write(path_name, response.into_inner().file_bytes).unwrap();

            {
                let mut flist = self.filelist.lock().unwrap();
                flist.push((filename,ip,ttr,expire_time,state));
            }
        }
        println!("Refresh Complete!");

        {
            let mut flist_expired = self.filelist_expired.lock().unwrap();
            flist_expired.clear();
        }

        

        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    async fn ttr_check(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<GenericReply>, Status> {

        let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();

        let mut files_expired = self.filelist.lock().unwrap().clone();
        files_expired.retain(|(_, _, _, expired_time, _)| *expired_time != 0 && expired_time < &current_time);
        for file in files_expired{
            let client_addr = format!("{}", file.1.clone());

            let mut client = FtpClient::connect(client_addr.clone()).await.unwrap();
            let request = tonic::Request::new(ReceiveRequest {
                filename: file.0.clone(),
            });
            let response = client.file_status(request).await?;
            let ttr = response.get_ref().ttr.clone();
            let state = response.get_ref().state.clone();
            let mut flist = self.filelist.lock().unwrap();
            if state == file.4 {
                flist.retain(|(filename_local,_, _, _, _)| *filename_local != file.0);
                let expire_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() + ttr;
                flist.push((file.0,file.1,ttr,expire_time,file.4));
            }
            else{
                let mut flist_expired = self.filelist_expired.lock().unwrap();
                let mut file_set = flist.clone();
                file_set.retain(|(filename_local,_, _, _, _)| *filename_local == file.0);
                let file_set = file_set.pop().unwrap();
                let file_set = (file_set.0, file_set.1);
                //Push to expired struct and remove from active struct
                flist_expired.push(file_set);
                flist.retain(|(filename_local,_, _, _, _)| *filename_local != file.0);
                let path_name = format!("{}/Downloaded/{}", env::current_dir()
                    .unwrap()
                    .display()
                    .to_string(), file.0.clone());
                
                fs::remove_file(path_name).unwrap_or_else(|_err|{
                    println!("Cannot delete file {}", file.0.clone());
                });
                print!("\nDeleted file: {} \
                \nThis file was removed from your Downloads folder since it was invalidated. \
                \nYou can re-download all invalid files with the \"Refresh\" command \
                \np2pft> ", file.0.clone());
                io::stdout().flush().unwrap();
            }
        }


        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    async fn file_status(
        &self,
        request: Request<ReceiveRequest>,
    ) -> Result<Response<FileStatusReply>, Status> {

        let filename = request.get_ref().filename.clone();
        let mut files = self.filelist.lock().unwrap().clone();
        files.retain(|(local_filename, _, _, _, _)| local_filename == &filename);
        let file = files.pop().unwrap();

        let reply = p2p_ft::FileStatusReply {
            ttr: file.2,
            state: file.4,
        };

        Ok(Response::new(reply))
    }

    async fn delete(
        &self,
        request: Request<DeleteRequest>,
    ) -> Result<Response<GenericReply>, Status> {

        let filename = request.get_ref().filename.clone();

        let mut flist = self.filelist.lock().unwrap();
        let mut flist_expired = self.filelist_expired.lock().unwrap();
        let mut file_set = flist.clone();
        file_set.retain(|(filename_local,_, _, _, _)| *filename_local == filename);
        let file_set = file_set.pop().unwrap();
        let file_set = (file_set.0, file_set.1);
        //Push to expired struct and remove from active struct
        flist_expired.push(file_set);
        flist.retain(|(filename_local,_, _, _, _)| *filename_local != filename);
        // Debug to print the full filelist that the FTP server has
        // println!("{:?}",flist);


        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    async fn state_change(
        &self,
        request: Request<StateUpdate>,
    ) -> Result<Response<GenericReply>, Status> {

        let filename = request.get_ref().filename.clone();
        let state = request.get_ref().state.clone();

        let mut flist = self.filelist.lock().unwrap();
        let mut file_set = flist.clone();
        file_set.retain(|(filename_local,_, _, _, _)| *filename_local == filename);
        let file_set = file_set.pop().unwrap();
        let file_set = (file_set.0, file_set.1, file_set.2, file_set.3, file_set.4);
        flist.retain(|(filename_local,_, _, _, _)| *filename_local != filename);
        flist.push((file_set.0, file_set.1, file_set.2, file_set.3, state));


        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }
    // ***************End Change***************
}
////////////////////////////////////////////////d///////////////////////////////////////////////////////////

///////////////////////////////////////////////////////////////////////////////////////////////////////////
/// Query Engine
/// Basic Structs for queries, query responses, and the query server
#[derive(Debug, Default, Clone)]
struct Query {
    id: u64,
    file: String,
    return_ip: String,
    seq_num: u32,
    ttl: u32,
    // ***************Start Change***************
    mess_type: u32
    // ***************End Change ***************
}

#[derive(Debug, Default, Clone)]
struct QueryResult {
    id: u64,
    file: String,
    return_ip: String,
    client_list: Vec<String>,
    seq_num: u32,
    // ***************Start Change***************
    mess_type: u32,
    states: Vec<u64>,
    // ***************End Change ***************
}

//Mutex guard on all the lists since it is multi-threaded
#[derive(Debug, Default)]
pub struct MyQueryEngine {
    query_list: AMutex<Vec<Query>>,
    query_response: AMutex<Vec<QueryResult>>,
    files: Mutex<Vec<(String, Vec<String>)>>,
    counter: AMutex<u32>,
    port: String,
    sent_query_list: AMutex<Vec<(u64,u32)>>,
    port_peer1: String,
}

//Most of these traits are not used by the client, but have to be implemented due to the RPC framework used.
#[tonic::async_trait]
impl QueryEngine for MyQueryEngine {
    //Not used by the client
    async fn receive(
        &self,
        // ***************Start Change***************
        _request: Request<QueryInitial>,
        // ***************End Change***************
    ) -> Result<Response<GenericReply>, Status> {

        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    //Not used by the client
    async fn broadcast(
        &self,
        _request: Request<QueryBroadcast>,
    ) -> Result<Response<GenericReply>, Status> {

        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    //Not used by the client
    async fn process(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<GenericReply>, Status> {

        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    //This is the function that the super-peer uses to return the query results
    async fn response(
        &self,
        request: Request<QueryResponse>,
    ) -> Result<Response<GenericReply>, Status> {

        let file = request.get_ref().file.clone();
        let client_id = request.get_ref().id.clone();
        let return_ip = request.get_ref().return_ip.clone();
        let client_list = request.get_ref().client_list.clone();
        let seq_num = request.get_ref().seq_num.clone();
        // ***************Start Change***************
        let mess_type = request.get_ref().mess_type.clone();
        let states = request.get_ref().states.clone();
        // ***************End Change ***************



        let query_result = QueryResult{
            file,
            id: client_id,
            return_ip,
            client_list,
            seq_num,
            // ***************Start Change***************
            mess_type,
            states
            // ***************End Change***************
        };

        let mut q_list = self.query_response.lock().await;
        q_list.push(query_result);
        // println!("Time: {:#?}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis());


        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    //Not used by the client
    async fn broadcast_response(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<GenericReply>, Status> {

        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    //This function reads all the query responses and returns a list of ips
    async fn response_read(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<QueryReply>, Status> {

        let query_list = self.query_response.lock().await;
        let q_list = query_list.clone();
        let mut ips_out = Vec::new();
        let mut states_out = Vec::new();

        if !q_list.is_empty() {
            for query in q_list {
                ips_out.push(query.client_list);
                states_out.push(query.states);

            }
        }

        let reply = p2p_ft::QueryReply{
            ips: ips_out.into_iter().flatten().collect::<Vec<String>>(),
            states: states_out.into_iter().flatten().collect::<Vec<u64>>(),
        };

        Ok(Response::new(reply))
    }

    // ***************Start Change***************
    //This function reads all the query responses and returns a list of invalidate messages
    async fn check(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<CheckReply>, Status> {

        let mut query_list = self.query_response.lock().await;
        let q_list = query_list.clone();
        let mut q_out = Vec::new();

        if !q_list.is_empty() {
            for query in q_list {
                if query.mess_type == 1 {
                   q_out.push(query.file); 
                }
            }
            query_list.retain(|q| q.mess_type != 1);

        }

        let reply = p2p_ft::CheckReply{
            filenames: q_out,
        };

        Ok(Response::new(reply))
    }
    // ***************End Change***************
    
    //This clears the query list prior to running a query. This eliminates any lingering query responses that arrive
    // after the client already processed the results.
    async fn clear(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<GenericReply>, Status> {

        let mut query_list = self.query_response.lock().await;

        query_list.clear();

        let reply = p2p_ft::GenericReply{
            reply: 1,
        };


        Ok(Response::new(reply))
    }

    //Not used by the client
    async fn cache_clear(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<GenericReply>, Status> {

        let reply = p2p_ft::GenericReply{
            reply: 1,
        };


        Ok(Response::new(reply))
    }
}
///////////////////////////////////////////////////////////////////////////////////////////////////////////
/// Main P2P Client Functions

// This defines the fiels that make up the client
#[derive(Debug)]
struct Client {
    id: u64,
    mac: String,
    dir: String,
    files: Vec<String>,
    state: u64,
    ip: String,
    ip_qe: String,
    ip_sp: String,
    port: String,
    file_states: Vec<(String, u64)>

}
impl Hash for Client {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.mac.hash(state);
        self.dir.hash(state);
    }
}
// This trait allows the client to deregister from the server when it closes
// This doesn't work when the client gets a SIG KILL "CTR+C"
// This is a trait so it doesnt have to be explicitly called at the end of the program. 
// It happens automatically when the client falls out of scope (when the program ends)
impl Drop for Client {
    fn drop(&mut self) {
        match deregister(&self){
            Ok(_) => (),
            Err(_) => {
                std::process::exit(1)
            }
        }
    }
}

impl Client{

    // This creates the client by pulling in environment values
    fn new() -> Client{
        let mut rng = rand::thread_rng();
        let mut tmp_client = Client{
            id: 0,
            mac: get_mac_address().unwrap().unwrap().to_string(),
            files: list_files_local(),
            dir: env::current_dir().unwrap()
                .display()
                .to_string()
                .split("/")
                .collect::<Vec<&str>>()
                .into_iter()
                .rev()
                .take(2)
                .rev()
                .collect::<Vec<&str>>()
                .join("/"),
            state: calculate_hash(&list_files_local()),
            ip: local_ip().unwrap().to_string(),
            ip_sp: local_ip().unwrap().to_string(),
            ip_qe: local_ip().unwrap().to_string(),
            port: rng.gen_range(20000..65000).to_string(),
            // ***************Start Change***************
            file_states: list_files_local()
                .into_iter()
                .map(|filename| {
                        let path_name = format!("{}/Owned/{}", env::current_dir().unwrap().display().to_string(), filename.clone());
                        let file = fs::read(path_name).unwrap();
                        let hash = calculate_hash(&file);
                        (filename, hash)
                    }
                    ).collect(),
            // ***************End Change***************
        };
        let ip_base = tmp_client.ip.clone();
        tmp_client.ip = format!("{}:{}", &ip_base, &tmp_client.port);
        tmp_client.port= rng.gen_range(20000..65000).to_string();
        tmp_client.ip_qe = format!("{}:{}", &ip_base, &tmp_client.port);
        tmp_client.id = calculate_hash(&tmp_client);
        tmp_client
    }

    // This allows for the client to update its info on the index server
    // The client maintains a hash value of its current files as its state
    // When the hash value changes it de-registers and then re-registers to the index server.
    fn update_state(&mut self){
        let current_state = calculate_hash(&list_files_local());
        if self.state != current_state{
            self.state = current_state;
            self.files = list_files_local();
            deregister(&self).unwrap();
            register(&self).unwrap();
        }
        // ***************Start Change***************
        let mut update_states = Vec::new();

        for (filename, hash) in self.file_states.clone(){
            let path_name = format!("{}/Owned/{}", env::current_dir().unwrap().display().to_string(), filename.clone());
            let file = fs::read(path_name).unwrap();
            let new_hash = calculate_hash(&file);
            if new_hash != hash {
                update_states.push((filename.clone(), new_hash));
                query(&self, filename.to_string(), 1).unwrap_or_else(|_err|{
                    println!("Server Error");
                });
            }
        }
        if !update_states.is_empty(){
            for (filename_change, hash) in update_states{
                state_change(&self, filename_change.clone(), hash).unwrap();
                self.file_states.retain(|(filename, _)| filename.ne(&filename_change));
                self.file_states.push((filename_change.to_string(), hash));
            }
        }
        // ***************End Change***************
    }


    // This registers the clients info to the index server
    fn register(&mut self){
        match register(&self){
            Ok(_) => (),
            Err(_) => {
                println!("\nUnable to connect to server at {}", self.ip_sp);
                println!("Closing program\n");
                std::process::exit(1)
            }
        }
    }
}

fn main() {
    // create the client
    let mut client = Client::new();

    let mut args = env::args();
    args.next();
    let pull = if let Some(design) = args.next(){
        if design.eq("pull"){
            true
        }
        else{
            false
        }
    }
    else{
        false
    };

    //This reads in the data from the configuration file based on the directory the binary is launched from
    //If the binary is launched from an incorrect location it will exit
    let mut config = Ini::new();
    match config.load("../../../helpers/all_config.ini"){
        Ok(path) => path,
        Err(_) => {
            println!("\nYou have to run the client in a peer folder under a super-peer");
            std::process::exit(1)
        }
    };
    let sp_port = config.get(&client.id.to_string(), "SP_PORT").unwrap();
    let ftp_port = config.get(&client.id.to_string(), "FTP_PORT").unwrap();
    let qe_port = config.get(&client.id.to_string(), "QE_PORT").unwrap();
    let ttr_value: u64 = config.get("COMMON", "TTR").unwrap().parse().unwrap();

    let ip_base = local_ip().unwrap().to_string();
    client.ip = format!("{}:{}", &ip_base, ftp_port);
    client.ip_qe = format!("{}:{}", &ip_base, qe_port);
    client.ip_sp = format!("{}:{}", &ip_base, sp_port);
    
    // spin up the client ftp server in a separate thread
    let client_addr = client.ip.clone();
    thread::spawn(move || {
        build_client_server(client_addr, ttr_value).unwrap();
    });

    // Start the query server in another thread
    let client_addr = client.ip_qe.clone();
    thread::spawn(move || {
        build_query_server(client_addr).unwrap();
    });

    // register the client to the index server
    client.register();

    // print the opening text of the program
    println!("\nWelcome to P2P File Transfer Client\n\n\
    Commands (case-insensitive): \n   \
    Help \n   \
    List \n   \
    Search <filename>\n   \
    Get <filename>\n   \
    Refresh \n   \
    Exit\n"
    );

    // create variables to allow for message passing between threads
    let (tx, rx) = mpsc::channel();
    let (tx2, rx2) = mpsc::channel();

    // send the user input collection to a separate thread. This allows for the update mechanism to run even if 
    // the program is waiting for user input
    thread::spawn(move || {
        loop {
            print!("p2pft> ");
            io::stdout().flush().unwrap();
            let mut command = String::new();

            io::stdin()
            .read_line(&mut command)
            .expect("Failed to read line");

            tx.send(command).unwrap();
            if let "TERM" = rx2.recv().unwrap(){
                break;
            }
        }
    });

    loop {
        // constantly check for a state change and update the server
        // a sleep function is added at the end of this loop to prevent CPU overuse with spin-lock
        client.update_state();
        if !pull{
            if let Ok(current_messages) = check(&client){
                if !current_messages.is_empty(){
                    for file in current_messages{
                        let file_path = format!("{}/Downloaded/{}", env::current_dir().unwrap().display().to_string(), file.clone());
                        if Path::new(&file_path).exists(){
                            remove_file(file, &client).unwrap();
                        }
                    }
                }
            }
        }
        else{
            ttr_check(&client).unwrap();

        }

        
        // Execute the follow if the user provides some input
        if let Ok(command) = rx.try_recv() {
            let command_list: Vec<&str> = command.split_whitespace().collect();

            // match user input to a specific command or print that the command is invalid. Each command
            // calls a function listed below
            if let Some(command) = command_list.first() { 
                match command.to_uppercase().trim() {
                        "EXIT" => {
                            // deregistering is a drop trait in the struct
                            tx2.send("TERM").unwrap();
                            println!("Deregistered from Server");
                            println!("Exiting");
                            break;
                        }
                        "HELP" => {
                            println!("\n   Help - Displays this message \n   \
                            List - List all files \n   \
                            Search <FILENAME> - Search for a file to download \n   \
                            Get <FILENAME> - Download a file \n   \
                            Info - View client info \n   \
                            Refresh - Redownloads all the expired files in the downloaded folder \n   \
                            Exit - Exit the program\n");
                        }
                        "INFO" => {
                            println!("{:#?}", client);
                        }
                        "LIST" => {
                            match list_files(&client){
                                Ok(_) => (),
                                Err(_) => {
                                    println!{"\nServer is offline. Closing program\n"};
                                    std::process::exit(1)
                                }
                            }
                        }
                        "REFRESH" => {
                            refresh(client.ip.clone()).unwrap();
                        }
                        "SEARCH" => {
                            let mut args = command_list.into_iter();
                            args.next();

                            if let Some(filename) = args.next() {
                                println!("Searching...");
                                // println!("Time: {:#?}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis());
                                // ***************Start Change***************
                                query(&client, filename.to_string(), 0).unwrap_or_else(|_err|{
                                    println!("Server Error");
                                    
                                });
                                //wait for all the super-peer to reply to the broadcasted message
                                thread::sleep(Duration::from_millis(300));
                                let dest_id_list = respond(&client).unwrap_or_else(|_err|{
                                    println!("\nServer Error. Use the \"List\" command to check files on server\n");
                                    (Vec::new(), Vec::new())
                                });
                                
                                if dest_id_list.0.len() > 0 {
                                    println!("\nAvailable Clients: {}", dest_id_list.0.len());
                                    // let mut states = dest_id_list.1.clone();
                                    // states.sort();
                                    // states.dedup();
                                    // let hit_rate = 1/states.len();
                                    // println!("Freshness Rate: {:?}", states);
                                }
                                else {
                                    println!("No Results");
                                    ()
                                }
                                // ***************End Change***************
                            }
                        }
                        "GET" => {
                            let mut args = command_list.into_iter();
                            args.next();

                            if let Some(filename) = args.next() {
                                // ***************Start Change***************
                                query(&client, filename.to_string(), 0).unwrap_or_else(|_err|{
                                    println!("Server Error");
                                    // ***************End Change***************
                                });
                                thread::sleep(Duration::from_millis(300));
                                let dest_id_list = respond(&client).unwrap_or_else(|_err|{
                                    println!("\nServer Error. Use the \"List\" command to check files on server\n");
                                    (Vec::new(), Vec::new())
                                });
                                match dest_id_list.0.first(){
                                    Some(ip) => {
                                        println!("Downloading from {}", ip);
                                        get(ip, filename.to_string(), &client.ip).unwrap_or_else(|_err|{
                                            println!("\nError downloading file. Please try again\n")
                                            });
                                    },
                                    None => {
                                        println!("No Results");
                                        ()
                                    },
                                }
                            } else {
                                println!("Please enter filename after search");
                            }
                        }
                        _ => println!("Invalid Command. Enter \"Help\" for options")
                    }
            }
            // let the user input thread run
            tx2.send("CONT").unwrap();
        }
        // This is to prevent spin-lock and burning up the CPU during idle. This only affects the auto-update mechanism and
        // not the search response time
        thread::sleep(Duration::from_millis(100));
    }
}

// This function creates and binds the client-client ftp server
#[tokio::main]
async fn build_client_server(addr: String, ttr: u64) -> Result<(), Box<dyn std::error::Error>> {
    
    // ***************Start Change***************
    let filelist_local = list_files_local();
    let mut filelist = Vec::new();

    for filename in filelist_local{
        let path_name = format!("{}/Owned/{}", env::current_dir().unwrap().display().to_string(), filename.clone());
        let file = fs::read(path_name).unwrap();
        let hash = calculate_hash(&file);
        let expired_time = 0;
        filelist.push((filename,"".to_string(),ttr.clone(),expired_time,hash));
    }

    let greeter = MyFTP{
        filelist: Mutex::new(filelist),
        filelist_expired: Mutex::new(Vec::new()),
        ip: format!("http://{}",addr.clone())
    };

    let addr = addr.parse()?;
    // ***************End Change***************
    Server::builder()
    .add_service(FtpServer::new(greeter))
    .serve(addr)
    .await?;
    
    Ok(())
}
// This function creates and binds the query-response server
#[tokio::main]
async fn build_query_server(addr: String) -> Result<(), Box<dyn std::error::Error>> {
    let addr = addr.parse()?;
    let greeter = MyQueryEngine{
        query_list: AMutex::new(Vec::new()),
        query_response: AMutex::new(Vec::new()),
        files: Mutex::new(Vec::new()),
        counter: AMutex::new(0), 
        port: "".to_string(),
        port_peer1: "".to_string(),
        sent_query_list: AMutex::new(Vec::new()),
    };

    Server::builder()
    .add_service(QueryEngineServer::new(greeter))
    .serve(addr)
    .await?;
    
    Ok(())
}
// This function requests a file list from the index server
#[tokio::main]
async fn list_files(client_local: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let client_addr = format!("http://{}", client_local.ip_sp); 
    let mut client = GreeterClient::connect(client_addr).await?;

    let request = tonic::Request::new(ListRequest {
        request: client_local.id,
    });

    let response = client.list_files(request).await?;

    println!("\nAvailable Files:\n{:#?}\n", response.get_ref().files.clone());
    Ok(())
}

// This function registers its info (IP, ID, files) to the index server
#[tokio::main]
async fn register(client_local: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let client_addr = format!("http://{}", client_local.ip_sp); 
    let mut client = GreeterClient::connect(client_addr).await?;

    let file_states = list_files_local()
        .into_iter()
        .map(|filename| {
                let file = {
                    if let Ok(file) = fs::read(format!("{}/Owned/{}", env::current_dir().unwrap().display().to_string(), filename.clone())) {
                        file
                    }
                    else{
                        fs::read(format!("{}/Downloaded/{}", env::current_dir().unwrap().display().to_string(), filename.clone())).unwrap()
                    };
                };
                let hash = calculate_hash(&file);
                hash
            }
            ).collect();

    let request = tonic::Request::new(RegisterRequest {
        hash: client_local.id,
        state: client_local.state,
        files: client_local.files.clone(), 
        ip: client_local.ip.clone(),
        ip_qe: client_local.ip_qe.clone(),
        file_states

    });
    let _response = client.register(request).await?;
    Ok(())
}

// This tells the index server to drop all client info
#[tokio::main]
async fn deregister(client_local: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let client_addr = format!("http://{}", client_local.ip_sp); 
    let mut client = GreeterClient::connect(client_addr).await?;

    let request = tonic::Request::new(DeregisterRequest {
        hash: client_local.id,
    });

    let _response = client.deregister(request).await?;
    Ok(())
}

// This clears the clients query queue and then sends a query to the super-peer
#[tokio::main]
async fn query(client_local: &Client, filename: String, mess_type: u32) -> Result<(), Box<dyn std::error::Error>> {

    let client_addr = format!("http://{}", client_local.ip_qe);
    let mut client = QueryEngineClient::connect(client_addr).await?;
    let request = p2p_ft::GenericReply {
        reply: 1,
    };
    client.clear(request).await?;

    let client_addr = format!("http://{}", client_local.ip_sp); 
    let mut client = GreeterClient::connect(client_addr).await?;

    // ***************Start Change***************
    let request = tonic::Request::new(QueryInitial {
        request: client_local.id,
        ip_qe: client_local.ip_qe.clone(),
        filename,
        mess_type,
    // ***************End Change***************
    });

    let _response = client.query(request).await?;
    Ok(())
}

//This pushes the results of a query into a query_response queue. The client will read all contents of this queue after it wakes
#[tokio::main]
async fn respond(client_local: &Client) -> Result<(Vec<String>, Vec<u64>), Box<dyn std::error::Error>> {

    let client_addr = format!("http://{}", client_local.ip_qe);
    let mut client = QueryEngineClient::connect(client_addr).await?;

    let request = p2p_ft::GenericReply {
        reply: 1,
    };
    let response = client.response_read(request).await?;
    Ok((response.get_ref().ips.clone(), response.get_ref().states.clone()))
}

// ***************Start Change***************
//This checks to see if there are any invalidate messages
#[tokio::main]
async fn check(client_local: &Client) -> Result<Vec<String>, Box<dyn std::error::Error>> {

    let client_addr = format!("http://{}", client_local.ip_qe);
    let mut client = QueryEngineClient::connect(client_addr).await?;

    let request = p2p_ft::GenericReply {
        reply: 1,
    };
    let response = client.check(request).await?;
    Ok(response.into_inner().filenames)
}

// This function removes a file after an invalidate message
#[tokio::main]
async fn remove_file(filename: String,client_local: &Client) -> Result<(), Box<dyn std::error::Error>> {

    let path_name = format!("{}/Downloaded/{}", env::current_dir()
        .unwrap()
        .display()
        .to_string(), &filename);
    
    fs::remove_file(path_name).unwrap_or_else(|_err|{
        println!("Cannot delete file {}", filename.clone());
    });
    print!("\nDeleted file: {} \
    \nThis file was removed from your Downloads folder since it was invalidated. \
    \nYou can re-download all expired files with the \"Refresh\" command \
    \np2pft> ", filename.clone());
    io::stdout().flush().unwrap();

    let local_client_ftp_addr = format!("http://{}", client_local.ip);
    let mut client = FtpClient::connect(local_client_ftp_addr).await?;
    let request = tonic::Request::new(DeleteRequest {
        filename,
    });
    let _response = client.delete(request).await?;

    Ok(())
    // ***************End Change ***************
}

// This is the only function that communicates with another client instead of the index server
// it sends its IP/Port info to another client that has a file it wants and then waits for the other client
// to send the file. This uses a separate thread with a thread pool to allow for multiple connections
#[tokio::main]
async fn get(addr: &String, filename: String, local_ip: &String) -> Result<(), Box<dyn std::error::Error>> {

    let client_addr = format!("http://{}", addr);
    let mut client = FtpClient::connect(client_addr.clone()).await?;
    println!("File Downloading");
    // ***************Start Change***************
    let path_name = format!("{}/Downloaded/{}", env::current_dir()
        .unwrap()
        .display()
        .to_string(), &filename);
    

    let request = tonic::Request::new(ReceiveRequest {
        filename: filename.clone(),
    });

    let response = client.receive(request).await?;
    let ip = response.get_ref().ip.clone();
    let ttr = response.get_ref().ttr.clone();
    let expire_time = response.get_ref().expire_time.clone();
    let state = response.get_ref().state.clone();
    fs::write(path_name, response.into_inner().file_bytes).unwrap();
    println!("File Downloaded");

    let expire_time = { match expire_time {
        0 => SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() + ttr,
        _ => expire_time,
        }
    };

    let local_client_ftp_addr = format!("http://{}", local_ip);
    let mut client = FtpClient::connect(local_client_ftp_addr).await?;
    let request = tonic::Request::new(UpdateRequest {
        filename,
        ip,
        ttr,
        expire_time,
        state,

    });
    let _response = client.update(request).await?;

    Ok(())
}

#[tokio::main]
async fn ttr_check(client_local: &Client) -> Result<(), Box<dyn std::error::Error>> {

    let client_addr = format!("http://{}", client_local.ip);
    let mut client = FtpClient::connect(client_addr.clone()).await?;

    let request = p2p_ft::GenericReply {
        reply: 1,
    };

    let _response = client.ttr_check(request).await?;
    Ok(())
}

#[tokio::main]
async fn state_change(client_local: &Client, filename: String, state: u64) -> Result<(), Box<dyn std::error::Error>> {

    let client_addr = format!("http://{}", client_local.ip);
    let mut client = FtpClient::connect(client_addr.clone()).await?;

    let request = p2p_ft::StateUpdate {
        filename,
        state,
    };

    let _response = client.state_change(request).await?;
    Ok(())
}

#[tokio::main]
async fn refresh(ip: String) -> Result<(), Box<dyn std::error::Error>> {

    let local_client_ftp_addr = format!("http://{}", ip);
    let mut client = FtpClient::connect(local_client_ftp_addr).await?;
    let request = p2p_ft::GenericReply {
        reply: 1,
    };
    let _response = client.refresh(request).await?;

    Ok(())
    // ***************End Change ***************
}
// This lists the local files in the working directory. It is used to register with the index server
fn list_files_local() -> Vec<String> {
    let mut entries = fs::read_dir("./Owned").unwrap()
        .filter(|e| e.as_ref().unwrap().file_type().unwrap().is_file())
        .map(|res| res.map(|e| e.file_name().into_string().unwrap()).unwrap())
        .collect::<Vec<String>>();
    let entries2 = fs::read_dir("./Downloaded").unwrap()
        .filter(|e| e.as_ref().unwrap().file_type().unwrap().is_file())
        .map(|res| res.map(|e| e.file_name().into_string().unwrap()).unwrap())
        .collect::<Vec<String>>();
    
    entries.extend(entries2);
    entries.sort();
    entries
}

// This hashs the client object (using the current working directory and MAC address) for a unique ID
fn calculate_hash<T: Hash>(t: &T) -> u64 {
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}
