use std::future::Future;

trait Execute {
    fn execute(&self) -> Result<u32, &'static str>;
}

struct Local;
struct Remote;

impl Execute for Local {
    fn execute(&self) -> Result<u32, &'static str> {
        Ok(1)
    }
}

impl Execute for Remote {
    fn execute(&self) -> Result<u32, &'static str> {
        Ok(2)
    }
}

macro_rules! invoke {
    ($value:expr) => {
        $value.execute()?
    };
}

fn dispatch<T: Execute>(value: &T) -> Result<u32, &'static str> {
    Ok(invoke!(value))
}

async fn async_dispatch<T, F>(value: T, callback: F) -> Result<u32, &'static str>
where
    T: Execute,
    F: FnOnce(u32) -> u32,
{
    let result = dispatch(&value)?;
    Ok(callback(result))
}

fn ambiguous(local: &Local, remote: &Remote) -> Result<u32, &'static str> {
    Ok(local.execute()? + remote.execute()?)
}

fn returns_future(
    local: Local,
) -> impl Future<Output = Result<u32, &'static str>> {
    async_dispatch(local, |value| value + 1)
}
