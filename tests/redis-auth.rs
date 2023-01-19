mod tests;
use redis::{acl::Rule, AsyncCommands};
use tests::suite;

use cuttlestore::Cuttlestore;
use tokio::test;

async fn add_user_to_redis() -> (String, String) {
    let redis = redis::Client::open("redis://127.0.0.1").unwrap();
    let mut conn = redis.get_async_connection().await.unwrap();

    let user = nanoid::nanoid!();
    let pass = nanoid::nanoid!();

    let rules = [
        Rule::On,
        Rule::AllKeys,
        Rule::AllCommands,
        Rule::AddPass(pass.clone()),
    ];

    let _: () = conn.acl_setuser_rules(&user, &rules).await.unwrap();

    (user, pass)
}

#[test]
async fn test_redis_auth() {
    let (user, pass) = add_user_to_redis().await;

    let store: Cuttlestore<String> =
        Cuttlestore::new(format!("redis://127.0.0.1?username={user}&password={pass}"))
            .await
            .unwrap();

    suite(&store).await;
}
