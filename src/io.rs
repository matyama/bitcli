use std::error::Error;
use std::str::FromStr;

use async_stream::try_stream;
use futures_util::TryStream;
use tokio::io::{self, AsyncBufReadExt as _, BufReader};

use crate::error::Result;

pub fn read_input<T>() -> impl TryStream<Item = Result<T>>
where
    T: FromStr + 'static,
    <T as FromStr>::Err: Error + Send + Sync,
{
    try_stream! {
        let mut stdin = BufReader::new(io::stdin());
        let mut buf = String::new();

        loop {
            let n = stdin.read_line(&mut buf).await?;

            if n == 0 {
                break;
            }

            yield buf.trim().parse::<T>().map_err(invalid_input)?;

            buf.clear();
        }
    }
}

#[inline]
fn invalid_input<E>(e: E) -> std::io::Error
where
    E: Into<Box<dyn Error + Send + Sync>>,
{
    std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
}
