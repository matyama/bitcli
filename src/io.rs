use std::error::Error;
use std::io::IsTerminal as _;
use std::os::fd::AsFd as _;
use std::str::FromStr;

use async_stream::try_stream;
use futures_util::TryStream;
use tokio::io::{self, AsyncBufReadExt as _, AsyncRead, BufReader};

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
        None
    } else {
        Some(try_read(stdin))
    }
}

fn try_read<T>(reader: impl AsyncRead + Unpin) -> impl TryStream<Item = Result<T>>
where
    T: FromStr + 'static,
    <T as FromStr>::Err: Error + Send + Sync,
{
    try_stream! {
        let mut reader = BufReader::new(reader);
        let mut buf = String::new();

        loop {
            let n = reader.read_line(&mut buf).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    use futures_util::StreamExt as _;
    use tokio_test::io::Builder;
    use url::Url;

    #[tokio::test]
    async fn empty_input() {
        let reader = Builder::new().build();
        let input = try_read::<Url>(reader).collect::<Vec<_>>().await;

        let expected: Vec<Url> = vec![];

        match input.into_iter().collect::<Result<Vec<_>>>() {
            Ok(actual) => assert_eq!(expected, actual),
            Err(error) => panic!("expected read to succeed with no data, got error: {error:?}"),
        }
    }

    #[tokio::test]
    async fn valid_input() {
        let reader = Builder::new()
            .read(b"https://example")
            .read(b".com\n")
            .read(b"http://example.com\n")
            .build();

        let input = try_read::<Url>(reader).collect::<Vec<_>>().await;

        let expected = vec![
            Url::parse("https://example.com").expect("valid URL"),
            Url::parse("http://example.com").expect("valid URL"),
        ];

        match input.into_iter().collect::<Result<Vec<_>>>() {
            Ok(actual) => assert_eq!(expected, actual),
            Err(error) => panic!("expected read to succeed with no data, got error: {error:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_input() {
        let reader = Builder::new()
            .read(b"https://example.com\n")
            .read(b"some invalid entry\n")
            .build();

        let input = try_read::<Url>(reader).collect::<Vec<_>>().await;
        let input = input.into_iter().collect::<Result<Vec<_>>>();

        if let Some(err) = input.expect_err("read should fail").source() {
            assert_eq!(
                err.downcast_ref::<std::io::Error>()
                    .expect("expected an std::io::Error")
                    .kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
    }

    #[tokio::test]
    async fn io_error() {
        let reader = Builder::new()
            .read(b"https://example")
            .read(b".com\n")
            .read_error(std::io::Error::other("IO error"))
            .build();

        let input = try_read::<Url>(reader).collect::<Vec<_>>().await;
        let input = input.into_iter().collect::<Result<Vec<_>>>();

        if let Some(err) = input.expect_err("read should fail").source() {
            assert!(err.to_string().contains("IO error"));
        }
    }
}
