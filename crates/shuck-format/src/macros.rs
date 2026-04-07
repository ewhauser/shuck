#[macro_export]
macro_rules! write {
    ($formatter:expr, [$($content:expr),* $(,)?]) => {{
        (|| -> ::std::result::Result<(), $crate::FormatError> {
            $( $crate::Format::fmt(&$content, $formatter)?; )*
            ::std::result::Result::<(), $crate::FormatError>::Ok(())
        })()
    }};
}

#[macro_export]
macro_rules! format {
    ($context:expr, [$($content:expr),* $(,)?]) => {{
        (|| -> ::std::result::Result<_, $crate::FormatError> {
            let mut formatter = $crate::Formatter::new($context);
            $crate::write!(&mut formatter, [$($content),*])?;
            ::std::result::Result::<_, $crate::FormatError>::Ok(formatter.finish())
        })()
    }};
}
