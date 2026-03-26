use mongodb::{ 
    Client, Collection, bson::{doc, oid::ObjectId}, options::{ClientOptions, Credential}
};
use serde::{ Deserialize, Serialize };
#[derive(Serialize, Deserialize, Debug)]
struct Countries {
    _id: ObjectId,
    name: String,
    code: String,
}
#[tokio::main]
async fn main() -> mongodb::error::Result<()> {
    let uri = "mongodb://127.0.0.1:27017/r310";
    let mut client_options = ClientOptions::parse(uri).await?;
    let default_cred = Credential::builder()
    .username("admin".to_string())
    .password("azerty".to_string())
    .source("admin".to_string())
    .build();
    client_options.credential = Some(default_cred);
    let client = Client::with_options(client_options)?;
    // Replace <T> with the <Document> or <Restaurant> type parameter
    let my_coll: Collection<Countries> = client
        .database("r310")
        .collection("countries");
    let mut result = my_coll.find(doc! {}).await?;
    //println!("{:#?}", result);
    while result.advance().await? {
        let record =  result.deserialize_current()?;
        println!("_id : {:?}", record._id);
        println!("name : {:?}", record.name);
        println!("code : {:?}", record.code);
    }
    Ok(())
}