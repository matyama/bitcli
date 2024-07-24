use std::error::Error;
use std::io::IsTerminal as _;
use std::os::fd::AsFd as _;
use std::str::FromStr;

use async_stream::try_stream;
use futures_util::TryStream;
use tokio::io::{self, AsyncBufReadExt as _, BufReader};

use crate::error::Result;

/// Read standard input as a [`TryStream`] of parsed lines of type `T`.
///
/// Returns `None` if the stdin handle does not refer to a terminal/tty.
pub fn read_input<T>() -> Option<impl TryStream<Item = Result<T>>>
where
    T: FromStr + 'static,
    <T as FromStr>::Err: Error + Send + Sync,
{
    let stdin = io::stdin();

    if stdin.as_fd().is_terminal() {
        return None;
    }

    Some(try_stream! {
        let mut stdin = BufReader::new(stdin);
        let mut buf = String::new();

        loop {
            let n = stdin.read_line(&mut buf).await?;

            if n == 0 {
                break;
            }

            yield buf.trim().parse::<T>().map_err(invalid_input)?;

            buf.clear();
        }
    })
}

#[inline]
fn invalid_input<E>(e: E) -> std::io::Error
where
    E: Into<Box<dyn Error + Send + Sync>>,
{
    std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
}
