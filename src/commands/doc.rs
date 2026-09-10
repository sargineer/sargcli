use super::Ctx;
use crate::cli::DocName;
use crate::error::Result;
use crate::output;

pub fn run(ctx: &mut Ctx, name: DocName) -> Result<()> {
    let headers = [("Accept".to_string(), "text/markdown, text/plain".to_string())];
    let r = ctx
        .client
        .ok(ctx.client.send("GET", name.path(), None, &headers)?)?;
    if ctx.json() {
        output::json(&serde_json::json!({ "path": name.path(), "body": r.body }));
    } else {
        println!("{}", r.body.trim_end());
    }
    Ok(())
}
