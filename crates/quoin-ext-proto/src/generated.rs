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
        /// The table `Call` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `Call` in the file `crates/quoin-ext-proto/schema/ext.fbs:17`
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
        pub struct Call {
            /// The field `op` in the table `Call`
            pub op: ::core::option::Option<::planus::alloc::string::String>,
            /// The field `arg` in the table `Call`
            pub arg: ::core::option::Option<::planus::alloc::string::String>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for Call {
            fn default() -> Self {
                Self {
                    op: ::core::default::Default::default(),
                    arg: ::core::default::Default::default(),
                }
            }
        }

        impl Call {
            /// Creates a [CallBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> CallBuilder<()> {
                CallBuilder(())
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

        impl ::planus::WriteAs<::planus::Offset<Call>> for Call {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Call> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<Call>> for Call {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Call>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<Call> for Call {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Call> {
                Call::create(builder, &self.op, &self.arg)
            }
        }

        /// Builder for serializing an instance of the [Call] type.
        ///
        /// Can be created using the [Call::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct CallBuilder<State>(State);

        impl CallBuilder<()> {
            /// Setter for the [`op` field](Call#structfield.op).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn op<T0>(self, value: T0) -> CallBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                CallBuilder((value,))
            }

            /// Sets the [`op` field](Call#structfield.op) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn op_as_null(self) -> CallBuilder<((),)> {
                self.op(())
            }
        }

        impl<T0> CallBuilder<(T0,)> {
            /// Setter for the [`arg` field](Call#structfield.arg).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn arg<T1>(self, value: T1) -> CallBuilder<(T0, T1)>
            where
                T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                let (v0,) = self.0;
                CallBuilder((v0, value))
            }

            /// Sets the [`arg` field](Call#structfield.arg) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn arg_as_null(self) -> CallBuilder<(T0, ())> {
                self.arg(())
            }
        }

        impl<T0, T1> CallBuilder<(T0, T1)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [Call].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<Call>
            where
                Self: ::planus::WriteAsOffset<Call>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAs<::planus::Offset<Call>> for CallBuilder<(T0, T1)>
        {
            type Prepared = ::planus::Offset<Call>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Call> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAsOptional<::planus::Offset<Call>> for CallBuilder<(T0, T1)>
        {
            type Prepared = ::planus::Offset<Call>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Call>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAsOffset<Call> for CallBuilder<(T0, T1)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Call> {
                let (v0, v1) = &self.0;
                Call::create(builder, v0, v1)
            }
        }

        /// Reference to a deserialized [Call].
        #[derive(Copy, Clone)]
        pub struct CallRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> CallRef<'a> {
            /// Getter for the [`op` field](Call#structfield.op).
            #[inline]
            pub fn op(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(0, "Call", "op")
            }

            /// Getter for the [`arg` field](Call#structfield.arg).
            #[inline]
            pub fn arg(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(1, "Call", "arg")
            }
        }

        impl<'a> ::core::fmt::Debug for CallRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("CallRef");
                if let ::core::option::Option::Some(field_op) = self.op().transpose() {
                    f.field("op", &field_op);
                }
                if let ::core::option::Option::Some(field_arg) = self.arg().transpose() {
                    f.field("arg", &field_arg);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<CallRef<'a>> for Call {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: CallRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    op: value.op()?.map(::core::convert::Into::into),
                    arg: value.arg()?.map(::core::convert::Into::into),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for CallRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for CallRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[CallRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<Call>> for Call {
            type Value = ::planus::Offset<Call>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<Call>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for CallRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[CallRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `CallReturn` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `CallReturn` in the file `crates/quoin-ext-proto/schema/ext.fbs:23`
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
        pub struct CallReturn {
            /// The field `result` in the table `CallReturn`
            pub result: ::core::option::Option<::planus::alloc::string::String>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for CallReturn {
            fn default() -> Self {
                Self {
                    result: ::core::default::Default::default(),
                }
            }
        }

        impl CallReturn {
            /// Creates a [CallReturnBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> CallReturnBuilder<()> {
                CallReturnBuilder(())
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

        impl ::planus::WriteAs<::planus::Offset<CallReturn>> for CallReturn {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<CallReturn> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<CallReturn>> for CallReturn {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<CallReturn>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<CallReturn> for CallReturn {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<CallReturn> {
                CallReturn::create(builder, &self.result)
            }
        }

        /// Builder for serializing an instance of the [CallReturn] type.
        ///
        /// Can be created using the [CallReturn::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct CallReturnBuilder<State>(State);

        impl CallReturnBuilder<()> {
            /// Setter for the [`result` field](CallReturn#structfield.result).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn result<T0>(self, value: T0) -> CallReturnBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                CallReturnBuilder((value,))
            }

            /// Sets the [`result` field](CallReturn#structfield.result) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn result_as_null(self) -> CallReturnBuilder<((),)> {
                self.result(())
            }
        }

        impl<T0> CallReturnBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [CallReturn].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<CallReturn>
            where
                Self: ::planus::WriteAsOffset<CallReturn>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAs<::planus::Offset<CallReturn>> for CallReturnBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<CallReturn>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<CallReturn> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOptional<::planus::Offset<CallReturn>> for CallReturnBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<CallReturn>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<CallReturn>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOffset<CallReturn> for CallReturnBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<CallReturn> {
                let (v0,) = &self.0;
                CallReturn::create(builder, v0)
            }
        }

        /// Reference to a deserialized [CallReturn].
        #[derive(Copy, Clone)]
        pub struct CallReturnRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> CallReturnRef<'a> {
            /// Getter for the [`result` field](CallReturn#structfield.result).
            #[inline]
            pub fn result(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(0, "CallReturn", "result")
            }
        }

        impl<'a> ::core::fmt::Debug for CallReturnRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("CallReturnRef");
                if let ::core::option::Option::Some(field_result) = self.result().transpose() {
                    f.field("result", &field_result);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<CallReturnRef<'a>> for CallReturn {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: CallReturnRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    result: value.result()?.map(::core::convert::Into::into),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for CallReturnRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for CallReturnRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[CallReturnRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<CallReturn>> for CallReturn {
            type Value = ::planus::Offset<CallReturn>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<CallReturn>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for CallReturnRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[CallReturnRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `MakeString` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `MakeString` in the file `crates/quoin-ext-proto/schema/ext.fbs:28`
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
        pub struct MakeString {
            /// The field `value` in the table `MakeString`
            pub value: ::core::option::Option<::planus::alloc::string::String>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for MakeString {
            fn default() -> Self {
                Self {
                    value: ::core::default::Default::default(),
                }
            }
        }

        impl MakeString {
            /// Creates a [MakeStringBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> MakeStringBuilder<()> {
                MakeStringBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_value: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            ) -> ::planus::Offset<Self> {
                let prepared_value = field_value.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_value.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_value) = prepared_value {
                            object_writer.write::<_, _, 4>(&prepared_value);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<MakeString>> for MakeString {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<MakeString> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<MakeString>> for MakeString {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<MakeString>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<MakeString> for MakeString {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<MakeString> {
                MakeString::create(builder, &self.value)
            }
        }

        /// Builder for serializing an instance of the [MakeString] type.
        ///
        /// Can be created using the [MakeString::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct MakeStringBuilder<State>(State);

        impl MakeStringBuilder<()> {
            /// Setter for the [`value` field](MakeString#structfield.value).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn value<T0>(self, value: T0) -> MakeStringBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                MakeStringBuilder((value,))
            }

            /// Sets the [`value` field](MakeString#structfield.value) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn value_as_null(self) -> MakeStringBuilder<((),)> {
                self.value(())
            }
        }

        impl<T0> MakeStringBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [MakeString].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<MakeString>
            where
                Self: ::planus::WriteAsOffset<MakeString>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAs<::planus::Offset<MakeString>> for MakeStringBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<MakeString>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<MakeString> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOptional<::planus::Offset<MakeString>> for MakeStringBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<MakeString>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<MakeString>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOffset<MakeString> for MakeStringBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<MakeString> {
                let (v0,) = &self.0;
                MakeString::create(builder, v0)
            }
        }

        /// Reference to a deserialized [MakeString].
        #[derive(Copy, Clone)]
        pub struct MakeStringRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> MakeStringRef<'a> {
            /// Getter for the [`value` field](MakeString#structfield.value).
            #[inline]
            pub fn value(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(0, "MakeString", "value")
            }
        }

        impl<'a> ::core::fmt::Debug for MakeStringRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("MakeStringRef");
                if let ::core::option::Option::Some(field_value) = self.value().transpose() {
                    f.field("value", &field_value);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<MakeStringRef<'a>> for MakeString {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: MakeStringRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    value: value.value()?.map(::core::convert::Into::into),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for MakeStringRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for MakeStringRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[MakeStringRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<MakeString>> for MakeString {
            type Value = ::planus::Offset<MakeString>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<MakeString>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for MakeStringRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[MakeStringRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `HandleToString` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `HandleToString` in the file `crates/quoin-ext-proto/schema/ext.fbs:33`
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
        pub struct HandleToString {
            /// The field `handle` in the table `HandleToString`
            pub handle: u64,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for HandleToString {
            fn default() -> Self {
                Self { handle: 0 }
            }
        }

        impl HandleToString {
            /// Creates a [HandleToStringBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> HandleToStringBuilder<()> {
                HandleToStringBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_handle: impl ::planus::WriteAsDefault<u64, u64>,
            ) -> ::planus::Offset<Self> {
                let prepared_handle = field_handle.prepare(builder, &0);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_handle.is_some() {
                    table_writer.write_entry::<u64>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_handle) = prepared_handle {
                            object_writer.write::<_, _, 8>(&prepared_handle);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<HandleToString>> for HandleToString {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HandleToString> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<HandleToString>> for HandleToString {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<HandleToString>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<HandleToString> for HandleToString {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HandleToString> {
                HandleToString::create(builder, self.handle)
            }
        }

        /// Builder for serializing an instance of the [HandleToString] type.
        ///
        /// Can be created using the [HandleToString::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct HandleToStringBuilder<State>(State);

        impl HandleToStringBuilder<()> {
            /// Setter for the [`handle` field](HandleToString#structfield.handle).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handle<T0>(self, value: T0) -> HandleToStringBuilder<(T0,)>
            where
                T0: ::planus::WriteAsDefault<u64, u64>,
            {
                HandleToStringBuilder((value,))
            }

            /// Sets the [`handle` field](HandleToString#structfield.handle) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handle_as_default(self) -> HandleToStringBuilder<(::planus::DefaultValue,)> {
                self.handle(::planus::DefaultValue)
            }
        }

        impl<T0> HandleToStringBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [HandleToString].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<HandleToString>
            where
                Self: ::planus::WriteAsOffset<HandleToString>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<u64, u64>>
            ::planus::WriteAs<::planus::Offset<HandleToString>> for HandleToStringBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<HandleToString>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HandleToString> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<u64, u64>>
            ::planus::WriteAsOptional<::planus::Offset<HandleToString>>
            for HandleToStringBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<HandleToString>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<HandleToString>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsDefault<u64, u64>> ::planus::WriteAsOffset<HandleToString>
            for HandleToStringBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HandleToString> {
                let (v0,) = &self.0;
                HandleToString::create(builder, v0)
            }
        }

        /// Reference to a deserialized [HandleToString].
        #[derive(Copy, Clone)]
        pub struct HandleToStringRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> HandleToStringRef<'a> {
            /// Getter for the [`handle` field](HandleToString#structfield.handle).
            #[inline]
            pub fn handle(&self) -> ::planus::Result<u64> {
                ::core::result::Result::Ok(
                    self.0.access(0, "HandleToString", "handle")?.unwrap_or(0),
                )
            }
        }

        impl<'a> ::core::fmt::Debug for HandleToStringRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("HandleToStringRef");
                f.field("handle", &self.handle());
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<HandleToStringRef<'a>> for HandleToString {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: HandleToStringRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    handle: ::core::convert::TryInto::try_into(value.handle()?)?,
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for HandleToStringRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for HandleToStringRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[HandleToStringRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<HandleToString>> for HandleToString {
            type Value = ::planus::Offset<HandleToString>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<HandleToString>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for HandleToStringRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[HandleToStringRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `Retain` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `Retain` in the file `crates/quoin-ext-proto/schema/ext.fbs:39`
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
        pub struct Retain {
            /// The field `handle` in the table `Retain`
            pub handle: u64,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for Retain {
            fn default() -> Self {
                Self { handle: 0 }
            }
        }

        impl Retain {
            /// Creates a [RetainBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> RetainBuilder<()> {
                RetainBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_handle: impl ::planus::WriteAsDefault<u64, u64>,
            ) -> ::planus::Offset<Self> {
                let prepared_handle = field_handle.prepare(builder, &0);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_handle.is_some() {
                    table_writer.write_entry::<u64>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_handle) = prepared_handle {
                            object_writer.write::<_, _, 8>(&prepared_handle);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<Retain>> for Retain {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Retain> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<Retain>> for Retain {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Retain>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<Retain> for Retain {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Retain> {
                Retain::create(builder, self.handle)
            }
        }

        /// Builder for serializing an instance of the [Retain] type.
        ///
        /// Can be created using the [Retain::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct RetainBuilder<State>(State);

        impl RetainBuilder<()> {
            /// Setter for the [`handle` field](Retain#structfield.handle).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handle<T0>(self, value: T0) -> RetainBuilder<(T0,)>
            where
                T0: ::planus::WriteAsDefault<u64, u64>,
            {
                RetainBuilder((value,))
            }

            /// Sets the [`handle` field](Retain#structfield.handle) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handle_as_default(self) -> RetainBuilder<(::planus::DefaultValue,)> {
                self.handle(::planus::DefaultValue)
            }
        }

        impl<T0> RetainBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [Retain].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<Retain>
            where
                Self: ::planus::WriteAsOffset<Retain>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<u64, u64>> ::planus::WriteAs<::planus::Offset<Retain>>
            for RetainBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<Retain>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Retain> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<u64, u64>>
            ::planus::WriteAsOptional<::planus::Offset<Retain>> for RetainBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<Retain>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Retain>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsDefault<u64, u64>> ::planus::WriteAsOffset<Retain>
            for RetainBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Retain> {
                let (v0,) = &self.0;
                Retain::create(builder, v0)
            }
        }

        /// Reference to a deserialized [Retain].
        #[derive(Copy, Clone)]
        pub struct RetainRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> RetainRef<'a> {
            /// Getter for the [`handle` field](Retain#structfield.handle).
            #[inline]
            pub fn handle(&self) -> ::planus::Result<u64> {
                ::core::result::Result::Ok(self.0.access(0, "Retain", "handle")?.unwrap_or(0))
            }
        }

        impl<'a> ::core::fmt::Debug for RetainRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("RetainRef");
                f.field("handle", &self.handle());
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<RetainRef<'a>> for Retain {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: RetainRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    handle: ::core::convert::TryInto::try_into(value.handle()?)?,
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for RetainRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for RetainRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[RetainRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<Retain>> for Retain {
            type Value = ::planus::Offset<Retain>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<Retain>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for RetainRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[RetainRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `Release` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `Release` in the file `crates/quoin-ext-proto/schema/ext.fbs:44`
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
        pub struct Release {
            /// The field `handles` in the table `Release`
            pub handles: ::core::option::Option<::planus::alloc::vec::Vec<u64>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for Release {
            fn default() -> Self {
                Self {
                    handles: ::core::default::Default::default(),
                }
            }
        }

        impl Release {
            /// Creates a [ReleaseBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> ReleaseBuilder<()> {
                ReleaseBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_handles: impl ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            ) -> ::planus::Offset<Self> {
                let prepared_handles = field_handles.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_handles.is_some() {
                    table_writer.write_entry::<::planus::Offset<[u64]>>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_handles) = prepared_handles {
                            object_writer.write::<_, _, 4>(&prepared_handles);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<Release>> for Release {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Release> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<Release>> for Release {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Release>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<Release> for Release {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Release> {
                Release::create(builder, &self.handles)
            }
        }

        /// Builder for serializing an instance of the [Release] type.
        ///
        /// Can be created using the [Release::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct ReleaseBuilder<State>(State);

        impl ReleaseBuilder<()> {
            /// Setter for the [`handles` field](Release#structfield.handles).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handles<T0>(self, value: T0) -> ReleaseBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            {
                ReleaseBuilder((value,))
            }

            /// Sets the [`handles` field](Release#structfield.handles) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handles_as_null(self) -> ReleaseBuilder<((),)> {
                self.handles(())
            }
        }

        impl<T0> ReleaseBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [Release].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<Release>
            where
                Self: ::planus::WriteAsOffset<Release>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>>
            ::planus::WriteAs<::planus::Offset<Release>> for ReleaseBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<Release>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Release> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>>
            ::planus::WriteAsOptional<::planus::Offset<Release>> for ReleaseBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<Release>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Release>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>>
            ::planus::WriteAsOffset<Release> for ReleaseBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Release> {
                let (v0,) = &self.0;
                Release::create(builder, v0)
            }
        }

        /// Reference to a deserialized [Release].
        #[derive(Copy, Clone)]
        pub struct ReleaseRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> ReleaseRef<'a> {
            /// Getter for the [`handles` field](Release#structfield.handles).
            #[inline]
            pub fn handles(
                &self,
            ) -> ::planus::Result<::core::option::Option<::planus::Vector<'a, u64>>> {
                self.0.access(0, "Release", "handles")
            }
        }

        impl<'a> ::core::fmt::Debug for ReleaseRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("ReleaseRef");
                if let ::core::option::Option::Some(field_handles) = self.handles().transpose() {
                    f.field("handles", &field_handles);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<ReleaseRef<'a>> for Release {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: ReleaseRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    handles: if let ::core::option::Option::Some(handles) = value.handles()? {
                        ::core::option::Option::Some(handles.to_vec()?)
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for ReleaseRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for ReleaseRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[ReleaseRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<Release>> for Release {
            type Value = ::planus::Offset<Release>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<Release>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for ReleaseRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[ReleaseRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `CallMethodOnHandle` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `CallMethodOnHandle` in the file `crates/quoin-ext-proto/schema/ext.fbs:52`
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
        pub struct CallMethodOnHandle {
            /// The field `receiver` in the table `CallMethodOnHandle`
            pub receiver: u64,
            /// The field `selector` in the table `CallMethodOnHandle`
            pub selector: ::core::option::Option<::planus::alloc::string::String>,
            /// The field `args` in the table `CallMethodOnHandle`
            pub args: ::core::option::Option<::planus::alloc::vec::Vec<u64>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for CallMethodOnHandle {
            fn default() -> Self {
                Self {
                    receiver: 0,
                    selector: ::core::default::Default::default(),
                    args: ::core::default::Default::default(),
                }
            }
        }

        impl CallMethodOnHandle {
            /// Creates a [CallMethodOnHandleBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> CallMethodOnHandleBuilder<()> {
                CallMethodOnHandleBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_receiver: impl ::planus::WriteAsDefault<u64, u64>,
                field_selector: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
                field_args: impl ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            ) -> ::planus::Offset<Self> {
                let prepared_receiver = field_receiver.prepare(builder, &0);
                let prepared_selector = field_selector.prepare(builder);
                let prepared_args = field_args.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<10> =
                    ::core::default::Default::default();
                if prepared_receiver.is_some() {
                    table_writer.write_entry::<u64>(0);
                }
                if prepared_selector.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(1);
                }
                if prepared_args.is_some() {
                    table_writer.write_entry::<::planus::Offset<[u64]>>(2);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_receiver) = prepared_receiver {
                            object_writer.write::<_, _, 8>(&prepared_receiver);
                        }
                        if let ::core::option::Option::Some(prepared_selector) = prepared_selector {
                            object_writer.write::<_, _, 4>(&prepared_selector);
                        }
                        if let ::core::option::Option::Some(prepared_args) = prepared_args {
                            object_writer.write::<_, _, 4>(&prepared_args);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<CallMethodOnHandle>> for CallMethodOnHandle {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallMethodOnHandle> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<CallMethodOnHandle>> for CallMethodOnHandle {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<CallMethodOnHandle>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<CallMethodOnHandle> for CallMethodOnHandle {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallMethodOnHandle> {
                CallMethodOnHandle::create(builder, self.receiver, &self.selector, &self.args)
            }
        }

        /// Builder for serializing an instance of the [CallMethodOnHandle] type.
        ///
        /// Can be created using the [CallMethodOnHandle::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct CallMethodOnHandleBuilder<State>(State);

        impl CallMethodOnHandleBuilder<()> {
            /// Setter for the [`receiver` field](CallMethodOnHandle#structfield.receiver).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn receiver<T0>(self, value: T0) -> CallMethodOnHandleBuilder<(T0,)>
            where
                T0: ::planus::WriteAsDefault<u64, u64>,
            {
                CallMethodOnHandleBuilder((value,))
            }

            /// Sets the [`receiver` field](CallMethodOnHandle#structfield.receiver) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn receiver_as_default(
                self,
            ) -> CallMethodOnHandleBuilder<(::planus::DefaultValue,)> {
                self.receiver(::planus::DefaultValue)
            }
        }

        impl<T0> CallMethodOnHandleBuilder<(T0,)> {
            /// Setter for the [`selector` field](CallMethodOnHandle#structfield.selector).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn selector<T1>(self, value: T1) -> CallMethodOnHandleBuilder<(T0, T1)>
            where
                T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                let (v0,) = self.0;
                CallMethodOnHandleBuilder((v0, value))
            }

            /// Sets the [`selector` field](CallMethodOnHandle#structfield.selector) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn selector_as_null(self) -> CallMethodOnHandleBuilder<(T0, ())> {
                self.selector(())
            }
        }

        impl<T0, T1> CallMethodOnHandleBuilder<(T0, T1)> {
            /// Setter for the [`args` field](CallMethodOnHandle#structfield.args).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn args<T2>(self, value: T2) -> CallMethodOnHandleBuilder<(T0, T1, T2)>
            where
                T2: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            {
                let (v0, v1) = self.0;
                CallMethodOnHandleBuilder((v0, v1, value))
            }

            /// Sets the [`args` field](CallMethodOnHandle#structfield.args) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn args_as_null(self) -> CallMethodOnHandleBuilder<(T0, T1, ())> {
                self.args(())
            }
        }

        impl<T0, T1, T2> CallMethodOnHandleBuilder<(T0, T1, T2)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [CallMethodOnHandle].
            #[inline]
            pub fn finish(
                self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallMethodOnHandle>
            where
                Self: ::planus::WriteAsOffset<CallMethodOnHandle>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<u64, u64>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T2: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
        > ::planus::WriteAs<::planus::Offset<CallMethodOnHandle>>
            for CallMethodOnHandleBuilder<(T0, T1, T2)>
        {
            type Prepared = ::planus::Offset<CallMethodOnHandle>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallMethodOnHandle> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<u64, u64>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T2: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
        > ::planus::WriteAsOptional<::planus::Offset<CallMethodOnHandle>>
            for CallMethodOnHandleBuilder<(T0, T1, T2)>
        {
            type Prepared = ::planus::Offset<CallMethodOnHandle>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<CallMethodOnHandle>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<u64, u64>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T2: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
        > ::planus::WriteAsOffset<CallMethodOnHandle> for CallMethodOnHandleBuilder<(T0, T1, T2)>
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallMethodOnHandle> {
                let (v0, v1, v2) = &self.0;
                CallMethodOnHandle::create(builder, v0, v1, v2)
            }
        }

        /// Reference to a deserialized [CallMethodOnHandle].
        #[derive(Copy, Clone)]
        pub struct CallMethodOnHandleRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> CallMethodOnHandleRef<'a> {
            /// Getter for the [`receiver` field](CallMethodOnHandle#structfield.receiver).
            #[inline]
            pub fn receiver(&self) -> ::planus::Result<u64> {
                ::core::result::Result::Ok(
                    self.0
                        .access(0, "CallMethodOnHandle", "receiver")?
                        .unwrap_or(0),
                )
            }

            /// Getter for the [`selector` field](CallMethodOnHandle#structfield.selector).
            #[inline]
            pub fn selector(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(1, "CallMethodOnHandle", "selector")
            }

            /// Getter for the [`args` field](CallMethodOnHandle#structfield.args).
            #[inline]
            pub fn args(
                &self,
            ) -> ::planus::Result<::core::option::Option<::planus::Vector<'a, u64>>> {
                self.0.access(2, "CallMethodOnHandle", "args")
            }
        }

        impl<'a> ::core::fmt::Debug for CallMethodOnHandleRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("CallMethodOnHandleRef");
                f.field("receiver", &self.receiver());
                if let ::core::option::Option::Some(field_selector) = self.selector().transpose() {
                    f.field("selector", &field_selector);
                }
                if let ::core::option::Option::Some(field_args) = self.args().transpose() {
                    f.field("args", &field_args);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<CallMethodOnHandleRef<'a>> for CallMethodOnHandle {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: CallMethodOnHandleRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    receiver: ::core::convert::TryInto::try_into(value.receiver()?)?,
                    selector: value.selector()?.map(::core::convert::Into::into),
                    args: if let ::core::option::Option::Some(args) = value.args()? {
                        ::core::option::Option::Some(args.to_vec()?)
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for CallMethodOnHandleRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for CallMethodOnHandleRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[CallMethodOnHandleRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<CallMethodOnHandle>> for CallMethodOnHandle {
            type Value = ::planus::Offset<CallMethodOnHandle>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<CallMethodOnHandle>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for CallMethodOnHandleRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[CallMethodOnHandleRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `HostOpReturn` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `HostOpReturn` in the file `crates/quoin-ext-proto/schema/ext.fbs:61`
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
        pub struct HostOpReturn {
            /// The field `handle` in the table `HostOpReturn`
            pub handle: u64,
            /// The field `str` in the table `HostOpReturn`
            pub str: ::core::option::Option<::planus::alloc::string::String>,
            /// The field `error` in the table `HostOpReturn`
            pub error: ::core::option::Option<::planus::alloc::string::String>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for HostOpReturn {
            fn default() -> Self {
                Self {
                    handle: 0,
                    str: ::core::default::Default::default(),
                    error: ::core::default::Default::default(),
                }
            }
        }

        impl HostOpReturn {
            /// Creates a [HostOpReturnBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> HostOpReturnBuilder<()> {
                HostOpReturnBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_handle: impl ::planus::WriteAsDefault<u64, u64>,
                field_str: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
                field_error: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            ) -> ::planus::Offset<Self> {
                let prepared_handle = field_handle.prepare(builder, &0);
                let prepared_str = field_str.prepare(builder);
                let prepared_error = field_error.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<10> =
                    ::core::default::Default::default();
                if prepared_handle.is_some() {
                    table_writer.write_entry::<u64>(0);
                }
                if prepared_str.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(1);
                }
                if prepared_error.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(2);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_handle) = prepared_handle {
                            object_writer.write::<_, _, 8>(&prepared_handle);
                        }
                        if let ::core::option::Option::Some(prepared_str) = prepared_str {
                            object_writer.write::<_, _, 4>(&prepared_str);
                        }
                        if let ::core::option::Option::Some(prepared_error) = prepared_error {
                            object_writer.write::<_, _, 4>(&prepared_error);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<HostOpReturn>> for HostOpReturn {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HostOpReturn> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<HostOpReturn>> for HostOpReturn {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<HostOpReturn>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<HostOpReturn> for HostOpReturn {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HostOpReturn> {
                HostOpReturn::create(builder, self.handle, &self.str, &self.error)
            }
        }

        /// Builder for serializing an instance of the [HostOpReturn] type.
        ///
        /// Can be created using the [HostOpReturn::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct HostOpReturnBuilder<State>(State);

        impl HostOpReturnBuilder<()> {
            /// Setter for the [`handle` field](HostOpReturn#structfield.handle).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handle<T0>(self, value: T0) -> HostOpReturnBuilder<(T0,)>
            where
                T0: ::planus::WriteAsDefault<u64, u64>,
            {
                HostOpReturnBuilder((value,))
            }

            /// Sets the [`handle` field](HostOpReturn#structfield.handle) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handle_as_default(self) -> HostOpReturnBuilder<(::planus::DefaultValue,)> {
                self.handle(::planus::DefaultValue)
            }
        }

        impl<T0> HostOpReturnBuilder<(T0,)> {
            /// Setter for the [`str` field](HostOpReturn#structfield.str).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn str<T1>(self, value: T1) -> HostOpReturnBuilder<(T0, T1)>
            where
                T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                let (v0,) = self.0;
                HostOpReturnBuilder((v0, value))
            }

            /// Sets the [`str` field](HostOpReturn#structfield.str) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn str_as_null(self) -> HostOpReturnBuilder<(T0, ())> {
                self.str(())
            }
        }

        impl<T0, T1> HostOpReturnBuilder<(T0, T1)> {
            /// Setter for the [`error` field](HostOpReturn#structfield.error).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn error<T2>(self, value: T2) -> HostOpReturnBuilder<(T0, T1, T2)>
            where
                T2: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                let (v0, v1) = self.0;
                HostOpReturnBuilder((v0, v1, value))
            }

            /// Sets the [`error` field](HostOpReturn#structfield.error) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn error_as_null(self) -> HostOpReturnBuilder<(T0, T1, ())> {
                self.error(())
            }
        }

        impl<T0, T1, T2> HostOpReturnBuilder<(T0, T1, T2)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [HostOpReturn].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<HostOpReturn>
            where
                Self: ::planus::WriteAsOffset<HostOpReturn>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<u64, u64>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T2: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAs<::planus::Offset<HostOpReturn>> for HostOpReturnBuilder<(T0, T1, T2)>
        {
            type Prepared = ::planus::Offset<HostOpReturn>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HostOpReturn> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<u64, u64>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T2: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAsOptional<::planus::Offset<HostOpReturn>>
            for HostOpReturnBuilder<(T0, T1, T2)>
        {
            type Prepared = ::planus::Offset<HostOpReturn>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<HostOpReturn>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<u64, u64>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T2: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAsOffset<HostOpReturn> for HostOpReturnBuilder<(T0, T1, T2)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HostOpReturn> {
                let (v0, v1, v2) = &self.0;
                HostOpReturn::create(builder, v0, v1, v2)
            }
        }

        /// Reference to a deserialized [HostOpReturn].
        #[derive(Copy, Clone)]
        pub struct HostOpReturnRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> HostOpReturnRef<'a> {
            /// Getter for the [`handle` field](HostOpReturn#structfield.handle).
            #[inline]
            pub fn handle(&self) -> ::planus::Result<u64> {
                ::core::result::Result::Ok(self.0.access(0, "HostOpReturn", "handle")?.unwrap_or(0))
            }

            /// Getter for the [`str` field](HostOpReturn#structfield.str).
            #[inline]
            pub fn str(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(1, "HostOpReturn", "str")
            }

            /// Getter for the [`error` field](HostOpReturn#structfield.error).
            #[inline]
            pub fn error(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(2, "HostOpReturn", "error")
            }
        }

        impl<'a> ::core::fmt::Debug for HostOpReturnRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("HostOpReturnRef");
                f.field("handle", &self.handle());
                if let ::core::option::Option::Some(field_str) = self.str().transpose() {
                    f.field("str", &field_str);
                }
                if let ::core::option::Option::Some(field_error) = self.error().transpose() {
                    f.field("error", &field_error);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<HostOpReturnRef<'a>> for HostOpReturn {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: HostOpReturnRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    handle: ::core::convert::TryInto::try_into(value.handle()?)?,
                    str: value.str()?.map(::core::convert::Into::into),
                    error: value.error()?.map(::core::convert::Into::into),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for HostOpReturnRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for HostOpReturnRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[HostOpReturnRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<HostOpReturn>> for HostOpReturn {
            type Value = ::planus::Offset<HostOpReturn>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<HostOpReturn>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for HostOpReturnRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[HostOpReturnRef]", "read_as_root", 0)
                })
            }
        }

        /// The union `Message` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Union `Message` in the file `crates/quoin-ext-proto/schema/ext.fbs:67`
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
        pub enum Message {
            /// The variant of type `Call` in the union `Message`
            Call(::planus::alloc::boxed::Box<self::Call>),

            /// The variant of type `CallReturn` in the union `Message`
            CallReturn(::planus::alloc::boxed::Box<self::CallReturn>),

            /// The variant of type `MakeString` in the union `Message`
            MakeString(::planus::alloc::boxed::Box<self::MakeString>),

            /// The variant of type `HandleToString` in the union `Message`
            HandleToString(::planus::alloc::boxed::Box<self::HandleToString>),

            /// The variant of type `Retain` in the union `Message`
            Retain(::planus::alloc::boxed::Box<self::Retain>),

            /// The variant of type `Release` in the union `Message`
            Release(::planus::alloc::boxed::Box<self::Release>),

            /// The variant of type `CallMethodOnHandle` in the union `Message`
            CallMethodOnHandle(::planus::alloc::boxed::Box<self::CallMethodOnHandle>),

            /// The variant of type `HostOpReturn` in the union `Message`
            HostOpReturn(::planus::alloc::boxed::Box<self::HostOpReturn>),
        }

        impl Message {
            /// Creates a [MessageBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> MessageBuilder<::planus::Uninitialized> {
                MessageBuilder(::planus::Uninitialized)
            }

            #[inline]
            pub fn create_call(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::Call>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(1, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_call_return(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::CallReturn>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(2, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_make_string(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::MakeString>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(3, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_handle_to_string(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::HandleToString>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(4, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_retain(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::Retain>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(5, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_release(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::Release>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(6, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_call_method_on_handle(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::CallMethodOnHandle>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(7, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_host_op_return(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::HostOpReturn>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(8, value.prepare(builder).downcast())
            }
        }

        impl ::planus::WriteAsUnion<Message> for Message {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Self> {
                match self {
                    Self::Call(value) => Self::create_call(builder, value),
                    Self::CallReturn(value) => Self::create_call_return(builder, value),
                    Self::MakeString(value) => Self::create_make_string(builder, value),
                    Self::HandleToString(value) => Self::create_handle_to_string(builder, value),
                    Self::Retain(value) => Self::create_retain(builder, value),
                    Self::Release(value) => Self::create_release(builder, value),
                    Self::CallMethodOnHandle(value) => {
                        Self::create_call_method_on_handle(builder, value)
                    }
                    Self::HostOpReturn(value) => Self::create_host_op_return(builder, value),
                }
            }
        }

        impl ::planus::WriteAsOptionalUnion<Message> for Message {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Self>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }

        /// Builder for serializing an instance of the [Message] type.
        ///
        /// Can be created using the [Message::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct MessageBuilder<T>(T);

        impl MessageBuilder<::planus::Uninitialized> {
            /// Creates an instance of the [`Call` variant](Message#variant.Call).
            #[inline]
            pub fn call<T>(self, value: T) -> MessageBuilder<::planus::Initialized<1, T>>
            where
                T: ::planus::WriteAsOffset<self::Call>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`CallReturn` variant](Message#variant.CallReturn).
            #[inline]
            pub fn call_return<T>(self, value: T) -> MessageBuilder<::planus::Initialized<2, T>>
            where
                T: ::planus::WriteAsOffset<self::CallReturn>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`MakeString` variant](Message#variant.MakeString).
            #[inline]
            pub fn make_string<T>(self, value: T) -> MessageBuilder<::planus::Initialized<3, T>>
            where
                T: ::planus::WriteAsOffset<self::MakeString>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`HandleToString` variant](Message#variant.HandleToString).
            #[inline]
            pub fn handle_to_string<T>(
                self,
                value: T,
            ) -> MessageBuilder<::planus::Initialized<4, T>>
            where
                T: ::planus::WriteAsOffset<self::HandleToString>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`Retain` variant](Message#variant.Retain).
            #[inline]
            pub fn retain<T>(self, value: T) -> MessageBuilder<::planus::Initialized<5, T>>
            where
                T: ::planus::WriteAsOffset<self::Retain>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`Release` variant](Message#variant.Release).
            #[inline]
            pub fn release<T>(self, value: T) -> MessageBuilder<::planus::Initialized<6, T>>
            where
                T: ::planus::WriteAsOffset<self::Release>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`CallMethodOnHandle` variant](Message#variant.CallMethodOnHandle).
            #[inline]
            pub fn call_method_on_handle<T>(
                self,
                value: T,
            ) -> MessageBuilder<::planus::Initialized<7, T>>
            where
                T: ::planus::WriteAsOffset<self::CallMethodOnHandle>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`HostOpReturn` variant](Message#variant.HostOpReturn).
            #[inline]
            pub fn host_op_return<T>(self, value: T) -> MessageBuilder<::planus::Initialized<8, T>>
            where
                T: ::planus::WriteAsOffset<self::HostOpReturn>,
            {
                MessageBuilder(::planus::Initialized(value))
            }
        }

        impl<const N: u8, T> MessageBuilder<::planus::Initialized<N, T>> {
            /// Finish writing the builder to get an [UnionOffset](::planus::UnionOffset) to a serialized [Message].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message>
            where
                Self: ::planus::WriteAsUnion<Message>,
            {
                ::planus::WriteAsUnion::prepare(&self, builder)
            }
        }

        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<1, T>>
        where
            T: ::planus::WriteAsOffset<self::Call>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(1, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<1, T>>
        where
            T: ::planus::WriteAsOffset<self::Call>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Message>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<2, T>>
        where
            T: ::planus::WriteAsOffset<self::CallReturn>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(2, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<2, T>>
        where
            T: ::planus::WriteAsOffset<self::CallReturn>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Message>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<3, T>>
        where
            T: ::planus::WriteAsOffset<self::MakeString>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(3, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<3, T>>
        where
            T: ::planus::WriteAsOffset<self::MakeString>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Message>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<4, T>>
        where
            T: ::planus::WriteAsOffset<self::HandleToString>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(4, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<4, T>>
        where
            T: ::planus::WriteAsOffset<self::HandleToString>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Message>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<5, T>>
        where
            T: ::planus::WriteAsOffset<self::Retain>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(5, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<5, T>>
        where
            T: ::planus::WriteAsOffset<self::Retain>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Message>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<6, T>>
        where
            T: ::planus::WriteAsOffset<self::Release>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(6, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<6, T>>
        where
            T: ::planus::WriteAsOffset<self::Release>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Message>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<7, T>>
        where
            T: ::planus::WriteAsOffset<self::CallMethodOnHandle>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(7, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<7, T>>
        where
            T: ::planus::WriteAsOffset<self::CallMethodOnHandle>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Message>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<8, T>>
        where
            T: ::planus::WriteAsOffset<self::HostOpReturn>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(8, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<8, T>>
        where
            T: ::planus::WriteAsOffset<self::HostOpReturn>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Message>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }

        /// Reference to a deserialized [Message].
        #[derive(Copy, Clone, Debug)]
        pub enum MessageRef<'a> {
            Call(self::CallRef<'a>),
            CallReturn(self::CallReturnRef<'a>),
            MakeString(self::MakeStringRef<'a>),
            HandleToString(self::HandleToStringRef<'a>),
            Retain(self::RetainRef<'a>),
            Release(self::ReleaseRef<'a>),
            CallMethodOnHandle(self::CallMethodOnHandleRef<'a>),
            HostOpReturn(self::HostOpReturnRef<'a>),
        }

        impl<'a> ::core::convert::TryFrom<MessageRef<'a>> for Message {
            type Error = ::planus::Error;

            fn try_from(value: MessageRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(match value {
                    MessageRef::Call(value) => Self::Call(::planus::alloc::boxed::Box::new(
                        ::core::convert::TryFrom::try_from(value)?,
                    )),

                    MessageRef::CallReturn(value) => {
                        Self::CallReturn(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    MessageRef::MakeString(value) => {
                        Self::MakeString(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    MessageRef::HandleToString(value) => {
                        Self::HandleToString(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    MessageRef::Retain(value) => Self::Retain(::planus::alloc::boxed::Box::new(
                        ::core::convert::TryFrom::try_from(value)?,
                    )),

                    MessageRef::Release(value) => Self::Release(::planus::alloc::boxed::Box::new(
                        ::core::convert::TryFrom::try_from(value)?,
                    )),

                    MessageRef::CallMethodOnHandle(value) => {
                        Self::CallMethodOnHandle(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    MessageRef::HostOpReturn(value) => {
                        Self::HostOpReturn(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }
                })
            }
        }

        impl<'a> ::planus::TableReadUnion<'a> for MessageRef<'a> {
            fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                tag: u8,
                field_offset: usize,
            ) -> ::core::result::Result<Self, ::planus::errors::ErrorKind> {
                match tag {
                    1 => ::core::result::Result::Ok(Self::Call(::planus::TableRead::from_buffer(
                        buffer,
                        field_offset,
                    )?)),
                    2 => ::core::result::Result::Ok(Self::CallReturn(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    3 => ::core::result::Result::Ok(Self::MakeString(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    4 => ::core::result::Result::Ok(Self::HandleToString(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    5 => ::core::result::Result::Ok(Self::Retain(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    6 => ::core::result::Result::Ok(Self::Release(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    7 => ::core::result::Result::Ok(Self::CallMethodOnHandle(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    8 => ::core::result::Result::Ok(Self::HostOpReturn(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    _ => {
                        ::core::result::Result::Err(::planus::errors::ErrorKind::UnknownUnionTag {
                            tag,
                        })
                    }
                }
            }
        }

        impl<'a> ::planus::VectorReadUnion<'a> for MessageRef<'a> {
            const VECTOR_NAME: &'static str = "[MessageRef]";
        }

        /// The table `Envelope` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `Envelope` in the file `crates/quoin-ext-proto/schema/ext.fbs:78`
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
        pub struct Envelope {
            /// The field `msg` in the table `Envelope`
            pub msg: ::core::option::Option<self::Message>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for Envelope {
            fn default() -> Self {
                Self {
                    msg: ::core::default::Default::default(),
                }
            }
        }

        impl Envelope {
            /// Creates a [EnvelopeBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> EnvelopeBuilder<()> {
                EnvelopeBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_msg: impl ::planus::WriteAsOptionalUnion<self::Message>,
            ) -> ::planus::Offset<Self> {
                let prepared_msg = field_msg.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<8> =
                    ::core::default::Default::default();
                if prepared_msg.is_some() {
                    table_writer.write_entry::<::planus::Offset<self::Message>>(1);
                }
                if prepared_msg.is_some() {
                    table_writer.write_entry::<u8>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_msg) = prepared_msg {
                            object_writer.write::<_, _, 4>(&prepared_msg.offset());
                        }
                        if let ::core::option::Option::Some(prepared_msg) = prepared_msg {
                            object_writer.write::<_, _, 1>(&prepared_msg.tag());
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<Envelope>> for Envelope {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Envelope> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<Envelope>> for Envelope {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Envelope>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<Envelope> for Envelope {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Envelope> {
                Envelope::create(builder, &self.msg)
            }
        }

        /// Builder for serializing an instance of the [Envelope] type.
        ///
        /// Can be created using the [Envelope::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct EnvelopeBuilder<State>(State);

        impl EnvelopeBuilder<()> {
            /// Setter for the [`msg` field](Envelope#structfield.msg).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn msg<T0>(self, value: T0) -> EnvelopeBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptionalUnion<self::Message>,
            {
                EnvelopeBuilder((value,))
            }

            /// Sets the [`msg` field](Envelope#structfield.msg) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn msg_as_null(self) -> EnvelopeBuilder<((),)> {
                self.msg(())
            }
        }

        impl<T0> EnvelopeBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [Envelope].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<Envelope>
            where
                Self: ::planus::WriteAsOffset<Envelope>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptionalUnion<self::Message>>
            ::planus::WriteAs<::planus::Offset<Envelope>> for EnvelopeBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<Envelope>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Envelope> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptionalUnion<self::Message>>
            ::planus::WriteAsOptional<::planus::Offset<Envelope>> for EnvelopeBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<Envelope>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<Envelope>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptionalUnion<self::Message>> ::planus::WriteAsOffset<Envelope>
            for EnvelopeBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Envelope> {
                let (v0,) = &self.0;
                Envelope::create(builder, v0)
            }
        }

        /// Reference to a deserialized [Envelope].
        #[derive(Copy, Clone)]
        pub struct EnvelopeRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> EnvelopeRef<'a> {
            /// Getter for the [`msg` field](Envelope#structfield.msg).
            #[inline]
            pub fn msg(&self) -> ::planus::Result<::core::option::Option<self::MessageRef<'a>>> {
                self.0.access_union(0, "Envelope", "msg")
            }
        }

        impl<'a> ::core::fmt::Debug for EnvelopeRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("EnvelopeRef");
                if let ::core::option::Option::Some(field_msg) = self.msg().transpose() {
                    f.field("msg", &field_msg);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<EnvelopeRef<'a>> for Envelope {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: EnvelopeRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    msg: if let ::core::option::Option::Some(msg) = value.msg()? {
                        ::core::option::Option::Some(::core::convert::TryInto::try_into(msg)?)
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for EnvelopeRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for EnvelopeRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[EnvelopeRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<Envelope>> for Envelope {
            type Value = ::planus::Offset<Envelope>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<Envelope>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for EnvelopeRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[EnvelopeRef]", "read_as_root", 0)
                })
            }
        }
    }
}
