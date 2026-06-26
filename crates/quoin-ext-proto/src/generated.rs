pub use root::*;

const _: () = ::planus::check_version_compatibility("planus-1.3.0");

/// The root namespace
///
/// Generated from these locations:
/// * File `crates/quoin-ext-proto/schema/ext.fbs`
#[no_implicit_prelude]
#[allow(clippy::needless_lifetimes)]
mod root {
    /// The namespace `quoin_ext_proto`
    ///
    /// Generated from these locations:
    /// * File `crates/quoin-ext-proto/schema/ext.fbs`
    pub mod quoin_ext_proto {
        /// The table `Request` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `Request` in the file `crates/quoin-ext-proto/schema/ext.fbs:11`
        #[derive(
            Clone,
            Debug,
            PartialEq,
            PartialOrd,
            Eq,
            Ord,
            Hash,
            ::serde::Serialize,
            ::serde::Deserialize,
        )]
        pub struct Request {
            /// The field `op` in the table `Request`
            pub op: ::core::option::Option<::planus::alloc::string::String>,
            /// The field `arg` in the table `Request`
            pub arg: ::core::option::Option<::planus::alloc::string::String>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for Request {
            fn default() -> Self {
                Self {
                    op: ::core::default::Default::default(),
                    arg: ::core::default::Default::default(),
                }
            }
        }

        impl Request {
            /// Creates a [RequestBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> RequestBuilder<()> {
                RequestBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_op: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
                field_arg: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            ) -> ::planus::Offset<Self> {
                let prepared_op = field_op.prepare(builder);
                let prepared_arg = field_arg.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<8> =
                    ::core::default::Default::default();
                if prepared_op.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(0);
                }
                if prepared_arg.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(1);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_op) = prepared_op {
                            object_writer.write::<_, _, 4>(&prepared_op);
                        }
                        if let ::core::option::Option::Some(prepared_arg) = prepared_arg {
                            object_writer.write::<_, _, 4>(&prepared_arg);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<Request>> for Request {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Request> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<Request>> for Request {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Request>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<Request> for Request {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Request> {
                Request::create(builder, &self.op, &self.arg)
            }
        }

        /// Builder for serializing an instance of the [Request] type.
        ///
        /// Can be created using the [Request::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct RequestBuilder<State>(State);

        impl RequestBuilder<()> {
            /// Setter for the [`op` field](Request#structfield.op).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn op<T0>(self, value: T0) -> RequestBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                RequestBuilder((value,))
            }

            /// Sets the [`op` field](Request#structfield.op) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn op_as_null(self) -> RequestBuilder<((),)> {
                self.op(())
            }
        }

        impl<T0> RequestBuilder<(T0,)> {
            /// Setter for the [`arg` field](Request#structfield.arg).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn arg<T1>(self, value: T1) -> RequestBuilder<(T0, T1)>
            where
                T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                let (v0,) = self.0;
                RequestBuilder((v0, value))
            }

            /// Sets the [`arg` field](Request#structfield.arg) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn arg_as_null(self) -> RequestBuilder<(T0, ())> {
                self.arg(())
            }
        }

        impl<T0, T1> RequestBuilder<(T0, T1)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [Request].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<Request>
            where
                Self: ::planus::WriteAsOffset<Request>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAs<::planus::Offset<Request>> for RequestBuilder<(T0, T1)>
        {
            type Prepared = ::planus::Offset<Request>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Request> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAsOptional<::planus::Offset<Request>> for RequestBuilder<(T0, T1)>
        {
            type Prepared = ::planus::Offset<Request>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Request>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAsOffset<Request> for RequestBuilder<(T0, T1)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Request> {
                let (v0, v1) = &self.0;
                Request::create(builder, v0, v1)
            }
        }

        /// Reference to a deserialized [Request].
        #[derive(Copy, Clone)]
        pub struct RequestRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> RequestRef<'a> {
            /// Getter for the [`op` field](Request#structfield.op).
            #[inline]
            pub fn op(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(0, "Request", "op")
            }

            /// Getter for the [`arg` field](Request#structfield.arg).
            #[inline]
            pub fn arg(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(1, "Request", "arg")
            }
        }

        impl<'a> ::core::fmt::Debug for RequestRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("RequestRef");
                if let ::core::option::Option::Some(field_op) = self.op().transpose() {
                    f.field("op", &field_op);
                }
                if let ::core::option::Option::Some(field_arg) = self.arg().transpose() {
                    f.field("arg", &field_arg);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<RequestRef<'a>> for Request {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: RequestRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    op: value.op()?.map(::core::convert::Into::into),
                    arg: value.arg()?.map(::core::convert::Into::into),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for RequestRef<'a> {
            #[inline]
            fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::core::result::Result<Self, ::planus::errors::ErrorKind> {
                ::core::result::Result::Ok(Self(::planus::table_reader::Table::from_buffer(
                    buffer, offset,
                )?))
            }
        }

        impl<'a> ::planus::VectorReadInner<'a> for RequestRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[RequestRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<Request>> for Request {
            type Value = ::planus::Offset<Request>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<Request>],
                bytes: *mut ::core::mem::MaybeUninit<u8>,
                buffer_position: u32,
            ) {
                let bytes = bytes as *mut [::core::mem::MaybeUninit<u8>; 4];
                for (i, v) in ::core::iter::Iterator::enumerate(values.iter()) {
                    ::planus::WriteAsPrimitive::write(
                        v,
                        ::planus::Cursor::new(unsafe { &mut *bytes.add(i) }),
                        buffer_position - (Self::STRIDE * i) as u32,
                    );
                }
            }
        }

        impl<'a> ::planus::ReadAsRoot<'a> for RequestRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[RequestRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `Response` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `Response` in the file `crates/quoin-ext-proto/schema/ext.fbs:17`
        #[derive(
            Clone,
            Debug,
            PartialEq,
            PartialOrd,
            Eq,
            Ord,
            Hash,
            ::serde::Serialize,
            ::serde::Deserialize,
        )]
        pub struct Response {
            /// The field `result` in the table `Response`
            pub result: ::core::option::Option<::planus::alloc::string::String>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for Response {
            fn default() -> Self {
                Self {
                    result: ::core::default::Default::default(),
                }
            }
        }

        impl Response {
            /// Creates a [ResponseBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> ResponseBuilder<()> {
                ResponseBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_result: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            ) -> ::planus::Offset<Self> {
                let prepared_result = field_result.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_result.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_result) = prepared_result {
                            object_writer.write::<_, _, 4>(&prepared_result);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<Response>> for Response {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Response> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<Response>> for Response {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Response>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<Response> for Response {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Response> {
                Response::create(builder, &self.result)
            }
        }

        /// Builder for serializing an instance of the [Response] type.
        ///
        /// Can be created using the [Response::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct ResponseBuilder<State>(State);

        impl ResponseBuilder<()> {
            /// Setter for the [`result` field](Response#structfield.result).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn result<T0>(self, value: T0) -> ResponseBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                ResponseBuilder((value,))
            }

            /// Sets the [`result` field](Response#structfield.result) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn result_as_null(self) -> ResponseBuilder<((),)> {
                self.result(())
            }
        }

        impl<T0> ResponseBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [Response].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<Response>
            where
                Self: ::planus::WriteAsOffset<Response>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAs<::planus::Offset<Response>> for ResponseBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<Response>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Response> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOptional<::planus::Offset<Response>> for ResponseBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<Response>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Response>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOffset<Response> for ResponseBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Response> {
                let (v0,) = &self.0;
                Response::create(builder, v0)
            }
        }

        /// Reference to a deserialized [Response].
        #[derive(Copy, Clone)]
        pub struct ResponseRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> ResponseRef<'a> {
            /// Getter for the [`result` field](Response#structfield.result).
            #[inline]
            pub fn result(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(0, "Response", "result")
            }
        }

        impl<'a> ::core::fmt::Debug for ResponseRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("ResponseRef");
                if let ::core::option::Option::Some(field_result) = self.result().transpose() {
                    f.field("result", &field_result);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<ResponseRef<'a>> for Response {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: ResponseRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    result: value.result()?.map(::core::convert::Into::into),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for ResponseRef<'a> {
            #[inline]
            fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::core::result::Result<Self, ::planus::errors::ErrorKind> {
                ::core::result::Result::Ok(Self(::planus::table_reader::Table::from_buffer(
                    buffer, offset,
                )?))
            }
        }

        impl<'a> ::planus::VectorReadInner<'a> for ResponseRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[ResponseRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<Response>> for Response {
            type Value = ::planus::Offset<Response>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<Response>],
                bytes: *mut ::core::mem::MaybeUninit<u8>,
                buffer_position: u32,
            ) {
                let bytes = bytes as *mut [::core::mem::MaybeUninit<u8>; 4];
                for (i, v) in ::core::iter::Iterator::enumerate(values.iter()) {
                    ::planus::WriteAsPrimitive::write(
                        v,
                        ::planus::Cursor::new(unsafe { &mut *bytes.add(i) }),
                        buffer_position - (Self::STRIDE * i) as u32,
                    );
                }
            }
        }

        impl<'a> ::planus::ReadAsRoot<'a> for ResponseRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[ResponseRef]", "read_as_root", 0)
                })
            }
        }
    }
}
