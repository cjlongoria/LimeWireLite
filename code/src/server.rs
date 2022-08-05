use tonic::{transport::Server, Request, Response, Status};
use p2p_ft::greeter_server::{Greeter, GreeterServer};
use p2p_ft::greeter_client::GreeterClient;
use p2p_ft::query_engine_client::QueryEngineClient;
use p2p_ft::query_engine_server::{QueryEngine, QueryEngineServer};
// ***************Start Change***************
use p2p_ft::{ListReply, ListRequest, RegisterRequest, RegisterReply, 
    DeregisterRequest, SearchRequest, SearchReply, GenericReply, 
    QueryResponse, QueryReply, QueryBroadcast, QueryInitial, CheckReply};
    // ***************End Change***************
use std::sync::Mutex;
use std::env;
// use tokio::sync::Mutex as AMutex;f
use unzip_n::unzip_n;
use tokio::time::{sleep, Duration};
use local_ip_address::local_ip;
use configparser::ini::Ini;
// use std::thread;
// use std::time::Duration;
// use std::sync::mpsc;
unzip_n!(5);
unzip_n!(4);


// This is used to pull in the RPC services/parameters
pub mod p2p_ft {
    tonic::include_proto!("p2pft"); // The string specified here must match the proto package name
}

// This is the client struct used to keep track of clients registering on the index server
#[derive(Debug)]
struct Client {
    id: u64,
    state: u64,
    files: Vec<String>,
    ip: String,
    ip_qe: String

}

// MyGreeter was the name I chose for the index server. This defines the fields that the server has. Since this 
// is multi-threaded and async each field is wrapped in a Mutex to prevent race conditions
#[derive(Debug, Default)]
pub struct MyGreeter {
    client_list: Mutex<Vec<Client>>,
    file_list: Mutex<Vec<(String, u64, String, String, u64)>>,
    qe_port: String,

}

//This is where most of the logic happens for the RPC index server. Each async function below is linked to an RPC service/function
#[tonic::async_trait]
impl Greeter for MyGreeter {

    //This function lists the files available for download. It removes duplicates and returns a vector of files to the client.
    //It prints on the server's terminal that it received a request from a client and lists the client id that requested a list
    async fn list_files(
        &self,
        request: Request<ListRequest>,
    ) -> Result<Response<ListReply>, Status> {
        println!("Got a full list request from Client: {}", request.into_inner().request);

        //lock server resource to prevent race conditions
        let id_file_tuple = self.file_list.lock().unwrap().clone();

        //get, sort, and remove duplications from the file list.
        let (_, _, mut current_files, _, _) = id_file_tuple.into_iter().unzip_n_vec();
        current_files.sort_unstable();
        current_files.dedup();

        //return the list
        let reply = p2p_ft::ListReply {
            files: current_files,
        };

        Ok(Response::new(reply))
    }

    //This function lists the clients that have a specific file. It returns a vector of client IPs and IDs that have the requested file.
    //It prints on the server's terminal that it received a request from a client and lists the client id that requested a search
    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchReply>, Status> {
        println!("Got a search request from Client: {}", request.get_ref().request);

        let filename = request.get_ref().filename.clone();
        // let addr = request.get_ref().ip_qe.clone();
        //lock server resource to prevent race conditions
        let mut search_results = self.file_list.lock().unwrap().clone();

        //filter list based on client search request
        search_results.retain(|(_requester_ip, _requester_id, file, _ip_qe, _)| *file == *filename);
        let (ip_search_results, id_search_results, file_search_results, ip_qe_search_results, state_search_results): 
                (Vec<_>, Vec<_>, Vec<_>, Vec<_>, Vec<_>) = {
            search_results
            .into_iter()
            .unzip_n_vec()
        };


        match file_search_results.first(){
            Some(file) => return  Ok(Response::new(p2p_ft::SearchReply {
                ips: ip_search_results,
                ids: id_search_results,
                file: file.to_string(),
                ips_qe: ip_qe_search_results,
                states: state_search_results
                })),
            None => return Err(Status::not_found("file not found"))
        }

    }

    async fn query(
        &self,
        // ***************Start Change***************
        request: Request<QueryInitial>,
        // ***************End Change***************
    ) -> Result<Response<GenericReply>, Status> {
        println!("Got a query request from Client: {}", request.get_ref().request);

        // ***************Start Change***************
        let query = tonic::Request::new(QueryInitial {
            request: request.get_ref().request.clone(),
            ip_qe: request.get_ref().ip_qe.clone(),
            filename: request.get_ref().filename.clone(),
            mess_type: request.get_ref().mess_type.clone(),
            // ***************End Change***************
        });


        let client_addr = format!("http://{}:{}", local_ip().unwrap().to_string(), self.qe_port);
        let mut client = QueryEngineClient::connect(client_addr).await.unwrap();
        client.receive(query).await.unwrap();

        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))

    }

    //This function runs when a client starts. It registers its info (IP, ID, and file list) to the server
    
    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterReply>, Status> {
        
        //client info getting pulled in and assigned locally
        let state = request.get_ref().state;
        let files = request.get_ref().files.clone();
        let id = request.get_ref().hash;
        let ip = request.get_ref().ip.clone();
        // ***************Start Change***************
        let ip_qe = request.get_ref().ip_qe.clone();
        let file_states = request.get_ref().file_states.clone();
        
        //locking server resources for write operations
        let mut clist = self.client_list.lock().unwrap();
        let mut flist = self.file_list.lock().unwrap();

        
        for i in 0..files.len(){
            flist.push((ip.clone(), id, files[i].clone(), ip_qe.clone(), file_states[i].clone()));
        }
        

        //create the client on the server and add it to the client list on the server
        let new_client = Client{id, ip, state, files, ip_qe};
        // ***************End Change***************
        println!("Registered Client: \n{:#?}", &new_client);
        clist.push(new_client);


        let reply = p2p_ft::RegisterReply {
            reply: 1,
        };
        
        //helper function to print the client details that just registered to the server
        print_summary(&clist, &flist);

        Ok(Response::new(reply))
    }
    
    //This is a light weight function that receives a client ID and removes all of its data from the server.
    async fn deregister(
        &self,
        request: Request<DeregisterRequest>,
    ) -> Result<Response<RegisterReply>, Status> {
        
        let id = request.get_ref().hash;
        println!("Deregistered Client: {:?}", request.into_inner().hash);

        // lock and filter out both the client and file lists on the server
        let mut flist = self.file_list.lock().unwrap();
        flist.retain(|(_, current_id, _, _, _)| *current_id != id);

        let mut clist = self.client_list.lock().unwrap();
        clist.retain(|client| client.id != id);


        let reply = p2p_ft::RegisterReply {
            reply: 1,
        };

        //helper function to print the client details that just deregistered from the server
        print_summary(&clist, &flist);

        Ok(Response::new(reply))
    }
}

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
    states: Vec<u64>
    // ***************End Change***************
}

//Mutex guard on all the lists since it is multi-threaded
#[derive(Debug, Default)]
pub struct MyQueryEngine {
    query_list: Mutex<Vec<Query>>,
    query_response: Mutex<Vec<QueryResult>>,
    files: Mutex<Vec<(String, Vec<String>)>>,
    sent_query_list: Mutex<Vec<(u64,u32,String,String)>>,
    counter: Mutex<u32>,
    port: String,
    port_peer1: String,
    port_peer2: String,
    port_peer3: String,
    port_peer4: String,
    port_peer5: String,
    port_peer6: String,
    port_peer7: String,
    qe_port: String,
}

#[tonic::async_trait]
impl QueryEngine for MyQueryEngine {
    //Receive and initial query from a client and increment the sequence counter.
    async fn receive(
        &self,
        // ***************Start Change***************
        // Change to initial query_message
        request: Request<QueryInitial>,
        // ***************End Change***************
    ) -> Result<Response<GenericReply>, Status> {

        let filename = request.get_ref().filename.clone();
        let client_id = request.get_ref().request.clone();
        let return_addr = request.get_ref().ip_qe.clone();
        let mess_type = request.get_ref().mess_type.clone();

        let mut num = self.counter.lock().unwrap();
        *num += 1;

        let new_query = Query{
            id: client_id,
            file: filename,
            return_ip: return_addr,
            seq_num: num.clone(),
            ttl: 3,
            // ***************Start Change***************
            mess_type,
            // ***************End Change***************
        };

        let mut q_list = self.query_list.lock().unwrap();

        q_list.push(new_query);

        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    //Broadcast the query to the neighbor super-peers. Drop the query if it has already been recieved
    async fn broadcast(
        &self,
        request: Request<QueryBroadcast>,
    ) -> Result<Response<GenericReply>, Status> {

        let id = request.get_ref().id.clone();
        let file = request.get_ref().file.clone();
        let return_ip = request.get_ref().return_ip.clone();
        let seq_num = request.get_ref().seq_num.clone();
        let ttl = request.get_ref().ttl.clone();
        // ***************Start Change***************
        let mess_type = request.get_ref().mess_type.clone();
        // ***************End Change ***************


        let new_query = Query{
            id,
            file,
            return_ip: return_ip.clone(),
            seq_num,
            ttl,
            // ***************Start Change***************
            mess_type,
            // ***************End Change ***************
        };

        let sent_query_list = self.sent_query_list.lock().unwrap();
        let sent_query_list_filtered = sent_query_list.clone();
        let query_id = (&id, &seq_num);


        if !sent_query_list_filtered.iter().any(|(id, seq, _, _)| (id, seq) == query_id){
            let mut q_list = self.query_list.lock().unwrap();
            q_list.push(new_query);
            println!("Pushed Query onto QE engine from broadcast");
        }
        let reply = p2p_ft::GenericReply {
            reply: 1,
        };
        

        Ok(Response::new(reply))
    }

    //This is run every 20ms and processes all the pending queries. This function goes through all the queries in 
    // The pending query list and broadcasts them out to the neighbor SP if it hasn't already. Which SP get the 
    // broadcasted message depend on the static configuration file read at startup. Any SP not connect have a port of '0' 
    // and are skipped over in the broadcast process. After completing the broadcast the SP will check its current files
    // (All the files registered from the leaf nodes) and return a query hit result if it has the requested file.
    async fn process(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<GenericReply>, Status> {

        let client_addr = format!("http://{}:{}", local_ip().unwrap().to_string(), self.port);
        let mut search_client = GreeterClient::connect(client_addr).await.unwrap();

        let sent_query_list = {
            let query_list = self.sent_query_list.lock().unwrap();
            query_list.clone()
        };
        
        let q_list = {
            let query_list = self.query_list.lock().unwrap();
            query_list.clone()
        };
        
        for query in q_list {
            
            let filename = query.file.clone();
            let client_id = query.id;
            let return_addr = query.return_ip.clone();
            let seq_num = query.seq_num;
            let query_id = (client_id, seq_num, return_addr.clone(), "".to_string());
            let ttl = query.ttl - 1;
            // ***************Start Change***************
            let mess_type = query.mess_type;
            // ***************End Change***************


            //check to see if query has already been pushed and check ttl value
            if !sent_query_list.contains(&query_id) && ttl > 0{
                {
                    let mut sent_query_list = self.sent_query_list.lock().unwrap();
                    sent_query_list.push(query_id);
                }
                let broadcast_return_addr = format!("{}:{}", local_ip().unwrap().to_string(), self.qe_port);

                

                //Since it is a static config file there is a list of 8 SP always, but when using the linear
                // design the non-neighbor SP will have a port of '0' and be skipped in the if..else statement for 
                // broadcasting
                let peers = {
                    vec![&self.port_peer1, &self.port_peer2, &self.port_peer3, 
                    &self.port_peer4, &self.port_peer5, &self.port_peer6, &self.port_peer7]
                };

                for peer in peers{

                    if !peer.eq("0"){
                        let new_query = tonic::Request::new(QueryBroadcast{
                            id: client_id,
                            file: filename.clone(),
                            return_ip: broadcast_return_addr.clone(),
                            seq_num,
                            ttl,
                            // ***************Start Change***************
                            mess_type,
                            // ***************End Change***************
                        });

                        let peer_addr = format!("http://{}:{}", local_ip().unwrap().to_string(), peer);
                        println!("broadcasting to {}", &peer_addr);
                        if let  Ok(mut peer_client) = QueryEngineClient::connect(peer_addr).await{
                            peer_client.broadcast(new_query).await?;
                            println!("Successfully broadcasted to peer");
                        }
                    }
                }

                println!("Pushed query to neighbors")
            }

            let request = tonic::Request::new(SearchRequest {
                request: client_id,
                ip_qe: return_addr.clone(),
                filename: filename.clone(),
            });

            if let Ok(response) = search_client.search(request).await{
                
                
                // ***************Start Change***************
                if mess_type == 0 {
                    let client_list = response.get_ref().ips.clone();
                    let states = response.get_ref().states.clone();
                    let query_response = tonic::Request::new(QueryResponse{
                        id: client_id,
                        file: filename,
                        return_ip: return_addr.clone(),
                        client_list,
                        states,
                        seq_num,
                        mess_type,
                    });

                    println!("Pushing a query response");
                    let client_addr = format!("http://{}", return_addr);
                    let mut client = QueryEngineClient::connect(client_addr).await.unwrap();
                    client.response(query_response).await?; 
                }
                else{
                    //the client ips are direct ftp connections not qe connections
                    let client_list = response.into_inner().ips_qe;
                    for client in client_list{
                        let query_response = tonic::Request::new(QueryResponse{
                            id: client_id,
                            file: filename.clone(),
                            return_ip: return_addr.clone(),
                            client_list: vec!("".to_string()),
                            states: vec!(0),
                            seq_num,
                            mess_type,
                        });

                        println!("Pushing a invalid message");
                        let client_addr = format!("http://{}", client);
                        let mut client = QueryEngineClient::connect(client_addr).await.unwrap();
                        client.response(query_response).await?; 
                    }
                }
                // ***************End Change***************
            }

        }

        //Finally clear out the pending query list so that the next time it is processed in 20ms there are no duplicates.
        let mut query_list = self.query_list.lock().unwrap();
        query_list.clear();


        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    //This adds a query response without incrementing the sequence number. Used to send back broadcast response
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
            // ***************End Change ***************
        };

        let mut q_list = self.query_response.lock().unwrap();
        q_list.push(query_result);


        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    //checks all the queries in the responded query queue and broadcasts them to the requestor via the function above. This function's
    //main purpose is to find the correct return address for back propagation.
    async fn broadcast_response(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<GenericReply>, Status> {

        let q_list ={
            let query_list = self.query_response.lock().unwrap();
            query_list.clone()
        };

        if !q_list.is_empty() {
            for query in q_list {
                let query_response = tonic::Request::new(QueryResponse{
                    id: query.id,
                    file: query.file.clone(),
                    return_ip: query.return_ip.clone(),
                    client_list: query.client_list.clone(),
                    seq_num: query.seq_num,
                    // ***************Start Change***************
                    mess_type: query.mess_type,
                    states: query.states.clone(),
                    // ***************End Change ***************

                });

                let mut sent_query_list ={
                    let query_list = self.sent_query_list.lock().unwrap();
                    query_list.clone()
                };
                if sent_query_list.iter().any(|(id, seq, _, _)| (id, seq) == (&query.id, &query.seq_num)){
                    sent_query_list.retain(|(existing_id, _, _, _)| *existing_id == query.id);
                    let (_, _, ip, _): (Vec<_>, Vec<_>, Vec<_>, Vec<_>) = {
                        sent_query_list
                        .into_iter()
                        .unzip_n_vec()
                    };

                    let return_addr = ip.first().unwrap();
                    let client_addr = format!("https://{}", return_addr);
                    println!("broadcasting response to final destination {}", &client_addr);
                    let mut client = QueryEngineClient::connect(client_addr).await.unwrap();
                    client.response(query_response).await.unwrap(); 
                }

                
            }
        }
        let mut query_list = self.query_response.lock().unwrap();
        query_list.clear();

        let reply = p2p_ft::GenericReply {
            reply: 1,
        };

        Ok(Response::new(reply))
    }

    //This is only used by the client to read all the query responses
    async fn response_read(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<QueryReply>, Status> {

        let reply = p2p_ft::QueryReply{
            ips: vec!("".to_string()),
            states: vec!(0)
        };

        Ok(Response::new(reply))
    }
    
    //Clear the pending query list
    async fn clear(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<GenericReply>, Status> {

        let mut query_list = self.query_response.lock().unwrap();

        query_list.clear();

        let reply = p2p_ft::GenericReply{
            reply: 1,
        };


        Ok(Response::new(reply))
    }

    //Clear the sent query list so there is no memory leakage
    async fn cache_clear(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<GenericReply>, Status> {

        let mut query_list = self.sent_query_list.lock().unwrap();
        
        let last_item = if query_list.len() > 0{
            query_list.pop()
        }
        else{
            None
        };

        query_list.clear();

        match last_item{
            Some(x) => query_list.push(x),
            None => (),
        }

        let reply = p2p_ft::GenericReply{
            reply: 1,
        };


        Ok(Response::new(reply))
    }

    // ***************Start Change***************
    //This isnt used by the server
    async fn check(
        &self,
        _request: Request<GenericReply>,
    ) -> Result<Response<CheckReply>, Status> {


        Err(Status::ok(""))
    }
    // ***************End Change***************
    
}
///////////////////////////////////////////////////////////////////////////////////////////////////////////


// This is the "main" program thread but all it does is set up the server on a local port. The tokio/async runtime
// provides multiple threads to handle client requests

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {


    //This reads in the data from the configuration file based on if there is a CLI argument
    //by default (if no arguments are provided or if something other than 'linear' is provided) it will choose a broadcast design
    
    let mut args = env::args();
    args.next();
    let config_path = if let Some(design) = args.next(){
        if design.eq("linear"){
            "../../helpers/linear_config.ini"
        }
        else{
            "../../helpers/all_config.ini"
        }
    }
    else{
        "../../helpers/all_config.ini"
    };
    //If the binary is launched from an incorrect location it will exit otherwise load in the configs
    let mut config = Ini::new();
    match config.load(config_path){
        Ok(path) => path,
        Err(_) => {
            println!("\nYou have to run the server in a super-peer folder");
            std::process::exit(1)
        }
    };
    let dir = env::current_dir()?
    .display()
    .to_string()
    .split("/")
    .collect::<Vec<&str>>()
    .last()
    .ok_or("")?
    .to_string();
    let sp_port = config.get(&dir, "PORT").unwrap();
    let qe_port = config.get(&dir, "QE_PORT").unwrap();
    let port_peer1 = config.get(&dir, "SP_Neighbor_1").unwrap();
    let port_peer2 = config.get(&dir, "SP_Neighbor_2").unwrap();
    let port_peer3 = config.get(&dir, "SP_Neighbor_3").unwrap();
    let port_peer4 = config.get(&dir, "SP_Neighbor_4").unwrap();
    let port_peer5 = config.get(&dir, "SP_Neighbor_5").unwrap();
    let port_peer6 = config.get(&dir, "SP_Neighbor_6").unwrap();
    let port_peer7 = config.get(&dir, "SP_Neighbor_7").unwrap();


    let addr = format!("{}:{}", local_ip()?.to_string(), &sp_port).parse()?;
    let greeter = MyGreeter{
        client_list: Mutex::new(Vec::new()),
        file_list: Mutex::new(Vec::new()),
        qe_port: qe_port.clone(),
    };

    //Spin off the super-peer indexing server in a different thread
    tokio::spawn(async move{
        match Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await{
            Ok(_) => (),
            Err(_) => {
                std::process::exit(1) 
            }
        }
    });
    
    //create a new thread to check the pending queries every 20ms and clear the sent query list
    // every 40 secs
    let qe_port_clone = qe_port.clone();
    let mut internal_counter = 0;
    tokio::spawn(async move{
        loop{
            
            sleep(Duration::from_millis(20)).await;
            let response = p2p_ft::GenericReply {
                reply: 1,
            };
            let client_addr = format!("http://{}:{}", local_ip().unwrap().to_string(), qe_port_clone);
            let mut client = QueryEngineClient::connect(client_addr).await.unwrap();
            client.process(response.clone()).await.unwrap_or_else(|_err|{
                Response::new(p2p_ft::GenericReply {
                reply: 1,
                })
            });
            client.broadcast_response(response.clone()).await.unwrap_or_else(|_err|{
                Response::new(p2p_ft::GenericReply {
                reply: 1,
                })
            });
            internal_counter += 1;
            if internal_counter == 2000{
                // clear sent_query_list
                client.cache_clear(response.clone()).await.unwrap_or_else(|_err|{
                    Response::new(p2p_ft::GenericReply {
                    reply: 1,
                    })
                });
                println!("Reset list of sent queries to super-peer neighbors");
                internal_counter = 0;
            };

        }
    });

    //build the query server from the loaded config file and spin up in a different thread
    let addr_qes = format!("{}:{}", local_ip()?.to_string(), qe_port.clone()).parse()?;
    let qes = MyQueryEngine{
        query_list: Mutex::new(Vec::new()),
        query_response: Mutex::new(Vec::new()),
        files: Mutex::new(Vec::new()),
        sent_query_list: Mutex::new(Vec::new()),
        counter: Mutex::new(0), 
        port: sp_port,
        qe_port,
        port_peer1,
        port_peer2,
        port_peer3,
        port_peer4,
        port_peer5,
        port_peer6,
        port_peer7,
    };

    Server::builder()
    .add_service(QueryEngineServer::new(qes))
    .serve(addr_qes)
    .await?;

    Ok(())
}

//This is a helper function for the register/deregister functions
fn print_summary(client_list: &Vec<Client>, file_list: &Vec<(String, u64, String, String, u64)>){

    println!("\n-----------------Server Summary------------------");
    println!("Active clients: {}", client_list.len());
    println!("Active files in index: {}", file_list.len());
    println!("-------------------------------------------------\n");

}
