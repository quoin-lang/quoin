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
        /// The enum `ArrowDType` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Enum `ArrowDType` in the file `crates/quoin-ext-proto/schema/ext.fbs:17`
        #[derive(
            Copy,
            Clone,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            ::serde::Serialize,
            ::serde::Deserialize,
        )]
        #[repr(u8)]
        pub enum ArrowDType {
            /// The variant `Float64` in the enum `ArrowDType`
            Float64 = 0,

            /// The variant `Int64` in the enum `ArrowDType`
            Int64 = 1,
        }

        impl ArrowDType {
            /// Array containing all valid variants of ArrowDType
            pub const ENUM_VALUES: [Self; 2] = [Self::Float64, Self::Int64];
        }

        impl ::core::convert::TryFrom<u8> for ArrowDType {
            type Error = ::planus::errors::UnknownEnumTagKind;
            #[inline]
            fn try_from(
                value: u8,
            ) -> ::core::result::Result<Self, ::planus::errors::UnknownEnumTagKind> {
                #[allow(clippy::match_single_binding)]
                match value {
                    0 => ::core::result::Result::Ok(ArrowDType::Float64),
                    1 => ::core::result::Result::Ok(ArrowDType::Int64),

                    _ => ::core::result::Result::Err(::planus::errors::UnknownEnumTagKind {
                        tag: value as i128,
                    }),
                }
            }
        }

        impl ::core::convert::From<ArrowDType> for u8 {
            #[inline]
            fn from(value: ArrowDType) -> Self {
                value as u8
            }
        }

        /// # Safety
        /// The Planus compiler correctly calculates `ALIGNMENT` and `SIZE`.
        unsafe impl ::planus::Primitive for ArrowDType {
            const ALIGNMENT: usize = 1;
            const SIZE: usize = 1;
        }

        impl ::planus::WriteAsPrimitive<ArrowDType> for ArrowDType {
            #[inline]
            fn write<const N: usize>(&self, cursor: ::planus::Cursor<'_, N>, buffer_position: u32) {
                (*self as u8).write(cursor, buffer_position);
            }
        }

        impl ::planus::WriteAs<ArrowDType> for ArrowDType {
            type Prepared = Self;

            #[inline]
            fn prepare(&self, _builder: &mut ::planus::Builder) -> ArrowDType {
                *self
            }
        }

        impl ::planus::WriteAsDefault<ArrowDType, ArrowDType> for ArrowDType {
            type Prepared = Self;

            #[inline]
            fn prepare(
                &self,
                _builder: &mut ::planus::Builder,
                default: &ArrowDType,
            ) -> ::core::option::Option<ArrowDType> {
                if self == default {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(*self)
                }
            }
        }

        impl ::planus::WriteAsOptional<ArrowDType> for ArrowDType {
            type Prepared = Self;

            #[inline]
            fn prepare(
                &self,
                _builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<ArrowDType> {
                ::core::option::Option::Some(*self)
            }
        }

        impl<'buf> ::planus::TableRead<'buf> for ArrowDType {
            #[inline]
            fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'buf>,
                offset: usize,
            ) -> ::core::result::Result<Self, ::planus::errors::ErrorKind> {
                let n: u8 = ::planus::TableRead::from_buffer(buffer, offset)?;
                ::core::result::Result::Ok(::core::convert::TryInto::try_into(n)?)
            }
        }

        impl<'buf> ::planus::VectorReadInner<'buf> for ArrowDType {
            type Error = ::planus::errors::UnknownEnumTag;
            const STRIDE: usize = 1;
            #[inline]
            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'buf>,
                offset: usize,
            ) -> ::core::result::Result<Self, ::planus::errors::UnknownEnumTag> {
                let value = unsafe { *buffer.buffer.get_unchecked(offset) };
                let value: ::core::result::Result<Self, _> =
                    ::core::convert::TryInto::try_into(value);
                value.map_err(|error_kind| {
                    error_kind.with_error_location(
                        "ArrowDType",
                        "VectorRead::from_buffer",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<ArrowDType> for ArrowDType {
            const STRIDE: usize = 1;

            type Value = Self;

            #[inline]
            fn prepare(&self, _builder: &mut ::planus::Builder) -> Self {
                *self
            }

            #[inline]
            unsafe fn write_values(
                values: &[Self],
                bytes: *mut ::core::mem::MaybeUninit<u8>,
                buffer_position: u32,
            ) {
                let bytes = bytes as *mut [::core::mem::MaybeUninit<u8>; 1];
                for (i, v) in ::core::iter::Iterator::enumerate(values.iter()) {
                    ::planus::WriteAsPrimitive::write(
                        v,
                        ::planus::Cursor::new(unsafe { &mut *bytes.add(i) }),
                        buffer_position - i as u32,
                    );
                }
            }
        }

        /// The table `ArrowArray` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `ArrowArray` in the file `crates/quoin-ext-proto/schema/ext.fbs:26`
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
        pub struct ArrowArray {
            /// The field `dtype` in the table `ArrowArray`
            pub dtype: self::ArrowDType,
            /// The field `length` in the table `ArrowArray`
            pub length: u64,
            /// The field `data` in the table `ArrowArray`
            pub data: ::core::option::Option<::planus::alloc::vec::Vec<u8>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for ArrowArray {
            fn default() -> Self {
                Self {
                    dtype: self::ArrowDType::Float64,
                    length: 0,
                    data: ::core::default::Default::default(),
                }
            }
        }

        impl ArrowArray {
            /// Creates a [ArrowArrayBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> ArrowArrayBuilder<()> {
                ArrowArrayBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_dtype: impl ::planus::WriteAsDefault<self::ArrowDType, self::ArrowDType>,
                field_length: impl ::planus::WriteAsDefault<u64, u64>,
                field_data: impl ::planus::WriteAsOptional<::planus::Offset<[u8]>>,
            ) -> ::planus::Offset<Self> {
                let prepared_dtype = field_dtype.prepare(builder, &self::ArrowDType::Float64);
                let prepared_length = field_length.prepare(builder, &0);
                let prepared_data = field_data.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<10> =
                    ::core::default::Default::default();
                if prepared_length.is_some() {
                    table_writer.write_entry::<u64>(1);
                }
                if prepared_data.is_some() {
                    table_writer.write_entry::<::planus::Offset<[u8]>>(2);
                }
                if prepared_dtype.is_some() {
                    table_writer.write_entry::<self::ArrowDType>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_length) = prepared_length {
                            object_writer.write::<_, _, 8>(&prepared_length);
                        }
                        if let ::core::option::Option::Some(prepared_data) = prepared_data {
                            object_writer.write::<_, _, 4>(&prepared_data);
                        }
                        if let ::core::option::Option::Some(prepared_dtype) = prepared_dtype {
                            object_writer.write::<_, _, 1>(&prepared_dtype);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<ArrowArray>> for ArrowArray {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<ArrowArray> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<ArrowArray>> for ArrowArray {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<ArrowArray>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<ArrowArray> for ArrowArray {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<ArrowArray> {
                ArrowArray::create(builder, self.dtype, self.length, &self.data)
            }
        }

        /// Builder for serializing an instance of the [ArrowArray] type.
        ///
        /// Can be created using the [ArrowArray::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct ArrowArrayBuilder<State>(State);

        impl ArrowArrayBuilder<()> {
            /// Setter for the [`dtype` field](ArrowArray#structfield.dtype).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn dtype<T0>(self, value: T0) -> ArrowArrayBuilder<(T0,)>
            where
                T0: ::planus::WriteAsDefault<self::ArrowDType, self::ArrowDType>,
            {
                ArrowArrayBuilder((value,))
            }

            /// Sets the [`dtype` field](ArrowArray#structfield.dtype) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn dtype_as_default(self) -> ArrowArrayBuilder<(::planus::DefaultValue,)> {
                self.dtype(::planus::DefaultValue)
            }
        }

        impl<T0> ArrowArrayBuilder<(T0,)> {
            /// Setter for the [`length` field](ArrowArray#structfield.length).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn length<T1>(self, value: T1) -> ArrowArrayBuilder<(T0, T1)>
            where
                T1: ::planus::WriteAsDefault<u64, u64>,
            {
                let (v0,) = self.0;
                ArrowArrayBuilder((v0, value))
            }

            /// Sets the [`length` field](ArrowArray#structfield.length) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn length_as_default(self) -> ArrowArrayBuilder<(T0, ::planus::DefaultValue)> {
                self.length(::planus::DefaultValue)
            }
        }

        impl<T0, T1> ArrowArrayBuilder<(T0, T1)> {
            /// Setter for the [`data` field](ArrowArray#structfield.data).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn data<T2>(self, value: T2) -> ArrowArrayBuilder<(T0, T1, T2)>
            where
                T2: ::planus::WriteAsOptional<::planus::Offset<[u8]>>,
            {
                let (v0, v1) = self.0;
                ArrowArrayBuilder((v0, v1, value))
            }

            /// Sets the [`data` field](ArrowArray#structfield.data) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn data_as_null(self) -> ArrowArrayBuilder<(T0, T1, ())> {
                self.data(())
            }
        }

        impl<T0, T1, T2> ArrowArrayBuilder<(T0, T1, T2)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [ArrowArray].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<ArrowArray>
            where
                Self: ::planus::WriteAsOffset<ArrowArray>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<self::ArrowDType, self::ArrowDType>,
            T1: ::planus::WriteAsDefault<u64, u64>,
            T2: ::planus::WriteAsOptional<::planus::Offset<[u8]>>,
        > ::planus::WriteAs<::planus::Offset<ArrowArray>> for ArrowArrayBuilder<(T0, T1, T2)>
        {
            type Prepared = ::planus::Offset<ArrowArray>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<ArrowArray> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<self::ArrowDType, self::ArrowDType>,
            T1: ::planus::WriteAsDefault<u64, u64>,
            T2: ::planus::WriteAsOptional<::planus::Offset<[u8]>>,
        > ::planus::WriteAsOptional<::planus::Offset<ArrowArray>>
            for ArrowArrayBuilder<(T0, T1, T2)>
        {
            type Prepared = ::planus::Offset<ArrowArray>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<ArrowArray>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<self::ArrowDType, self::ArrowDType>,
            T1: ::planus::WriteAsDefault<u64, u64>,
            T2: ::planus::WriteAsOptional<::planus::Offset<[u8]>>,
        > ::planus::WriteAsOffset<ArrowArray> for ArrowArrayBuilder<(T0, T1, T2)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<ArrowArray> {
                let (v0, v1, v2) = &self.0;
                ArrowArray::create(builder, v0, v1, v2)
            }
        }

        /// Reference to a deserialized [ArrowArray].
        #[derive(Copy, Clone)]
        pub struct ArrowArrayRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> ArrowArrayRef<'a> {
            /// Getter for the [`dtype` field](ArrowArray#structfield.dtype).
            #[inline]
            pub fn dtype(&self) -> ::planus::Result<self::ArrowDType> {
                ::core::result::Result::Ok(
                    self.0
                        .access(0, "ArrowArray", "dtype")?
                        .unwrap_or(self::ArrowDType::Float64),
                )
            }

            /// Getter for the [`length` field](ArrowArray#structfield.length).
            #[inline]
            pub fn length(&self) -> ::planus::Result<u64> {
                ::core::result::Result::Ok(self.0.access(1, "ArrowArray", "length")?.unwrap_or(0))
            }

            /// Getter for the [`data` field](ArrowArray#structfield.data).
            #[inline]
            pub fn data(&self) -> ::planus::Result<::core::option::Option<&'a [u8]>> {
                self.0.access(2, "ArrowArray", "data")
            }
        }

        impl<'a> ::core::fmt::Debug for ArrowArrayRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("ArrowArrayRef");
                f.field("dtype", &self.dtype());
                f.field("length", &self.length());
                if let ::core::option::Option::Some(field_data) = self.data().transpose() {
                    f.field("data", &field_data);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<ArrowArrayRef<'a>> for ArrowArray {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: ArrowArrayRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    dtype: ::core::convert::TryInto::try_into(value.dtype()?)?,
                    length: ::core::convert::TryInto::try_into(value.length()?)?,
                    data: value.data()?.map(|v| v.to_vec()),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for ArrowArrayRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for ArrowArrayRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[ArrowArrayRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<ArrowArray>> for ArrowArray {
            type Value = ::planus::Offset<ArrowArray>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<ArrowArray>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for ArrowArrayRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[ArrowArrayRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvNull` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvNull` in the file `crates/quoin-ext-proto/schema/ext.fbs:36`
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
        pub struct DvNull {}

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvNull {
            fn default() -> Self {
                Self {}
            }
        }

        impl DvNull {
            /// Creates a [DvNullBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvNullBuilder<()> {
                DvNullBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(builder: &mut ::planus::Builder) -> ::planus::Offset<Self> {
                let table_writer: ::planus::table_writer::TableWriter<4> =
                    ::core::default::Default::default();
                unsafe {
                    table_writer.finish(builder, |_table_writer| {});
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvNull>> for DvNull {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvNull> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvNull>> for DvNull {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvNull>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvNull> for DvNull {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvNull> {
                DvNull::create(builder)
            }
        }

        /// Builder for serializing an instance of the [DvNull] type.
        ///
        /// Can be created using the [DvNull::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvNullBuilder<State>(State);

        impl DvNullBuilder<()> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvNull].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvNull>
            where
                Self: ::planus::WriteAsOffset<DvNull>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvNull>> for DvNullBuilder<()> {
            type Prepared = ::planus::Offset<DvNull>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvNull> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvNull>> for DvNullBuilder<()> {
            type Prepared = ::planus::Offset<DvNull>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvNull>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvNull> for DvNullBuilder<()> {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvNull> {
                DvNull::create(builder)
            }
        }

        /// Reference to a deserialized [DvNull].
        #[derive(Copy, Clone)]
        pub struct DvNullRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvNullRef<'a> {}

        impl<'a> ::core::fmt::Debug for DvNullRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvNullRef");

                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvNullRef<'a>> for DvNull {
            type Error = ::planus::Error;

            fn try_from(_value: DvNullRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {})
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvNullRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvNullRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[DvNullRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvNull>> for DvNull {
            type Value = ::planus::Offset<DvNull>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvNull>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvNullRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvNullRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvBool` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvBool` in the file `crates/quoin-ext-proto/schema/ext.fbs:37`
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
        pub struct DvBool {
            /// The field `v` in the table `DvBool`
            pub v: bool,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvBool {
            fn default() -> Self {
                Self { v: false }
            }
        }

        impl DvBool {
            /// Creates a [DvBoolBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvBoolBuilder<()> {
                DvBoolBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_v: impl ::planus::WriteAsDefault<bool, bool>,
            ) -> ::planus::Offset<Self> {
                let prepared_v = field_v.prepare(builder, &false);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_v.is_some() {
                    table_writer.write_entry::<bool>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_v) = prepared_v {
                            object_writer.write::<_, _, 1>(&prepared_v);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvBool>> for DvBool {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBool> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvBool>> for DvBool {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvBool>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvBool> for DvBool {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBool> {
                DvBool::create(builder, self.v)
            }
        }

        /// Builder for serializing an instance of the [DvBool] type.
        ///
        /// Can be created using the [DvBool::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvBoolBuilder<State>(State);

        impl DvBoolBuilder<()> {
            /// Setter for the [`v` field](DvBool#structfield.v).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v<T0>(self, value: T0) -> DvBoolBuilder<(T0,)>
            where
                T0: ::planus::WriteAsDefault<bool, bool>,
            {
                DvBoolBuilder((value,))
            }

            /// Sets the [`v` field](DvBool#structfield.v) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v_as_default(self) -> DvBoolBuilder<(::planus::DefaultValue,)> {
                self.v(::planus::DefaultValue)
            }
        }

        impl<T0> DvBoolBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvBool].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBool>
            where
                Self: ::planus::WriteAsOffset<DvBool>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<bool, bool>> ::planus::WriteAs<::planus::Offset<DvBool>>
            for DvBoolBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvBool>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBool> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<bool, bool>>
            ::planus::WriteAsOptional<::planus::Offset<DvBool>> for DvBoolBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvBool>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvBool>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsDefault<bool, bool>> ::planus::WriteAsOffset<DvBool>
            for DvBoolBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBool> {
                let (v0,) = &self.0;
                DvBool::create(builder, v0)
            }
        }

        /// Reference to a deserialized [DvBool].
        #[derive(Copy, Clone)]
        pub struct DvBoolRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvBoolRef<'a> {
            /// Getter for the [`v` field](DvBool#structfield.v).
            #[inline]
            pub fn v(&self) -> ::planus::Result<bool> {
                ::core::result::Result::Ok(self.0.access(0, "DvBool", "v")?.unwrap_or(false))
            }
        }

        impl<'a> ::core::fmt::Debug for DvBoolRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvBoolRef");
                f.field("v", &self.v());
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvBoolRef<'a>> for DvBool {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DvBoolRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    v: ::core::convert::TryInto::try_into(value.v()?)?,
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvBoolRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvBoolRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[DvBoolRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvBool>> for DvBool {
            type Value = ::planus::Offset<DvBool>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvBool>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvBoolRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvBoolRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvInt` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvInt` in the file `crates/quoin-ext-proto/schema/ext.fbs:38`
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
        pub struct DvInt {
            /// The field `v` in the table `DvInt`
            pub v: i64,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvInt {
            fn default() -> Self {
                Self { v: 0 }
            }
        }

        impl DvInt {
            /// Creates a [DvIntBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvIntBuilder<()> {
                DvIntBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_v: impl ::planus::WriteAsDefault<i64, i64>,
            ) -> ::planus::Offset<Self> {
                let prepared_v = field_v.prepare(builder, &0);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_v.is_some() {
                    table_writer.write_entry::<i64>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_v) = prepared_v {
                            object_writer.write::<_, _, 8>(&prepared_v);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvInt>> for DvInt {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvInt> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvInt>> for DvInt {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvInt>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvInt> for DvInt {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvInt> {
                DvInt::create(builder, self.v)
            }
        }

        /// Builder for serializing an instance of the [DvInt] type.
        ///
        /// Can be created using the [DvInt::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvIntBuilder<State>(State);

        impl DvIntBuilder<()> {
            /// Setter for the [`v` field](DvInt#structfield.v).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v<T0>(self, value: T0) -> DvIntBuilder<(T0,)>
            where
                T0: ::planus::WriteAsDefault<i64, i64>,
            {
                DvIntBuilder((value,))
            }

            /// Sets the [`v` field](DvInt#structfield.v) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v_as_default(self) -> DvIntBuilder<(::planus::DefaultValue,)> {
                self.v(::planus::DefaultValue)
            }
        }

        impl<T0> DvIntBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvInt].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvInt>
            where
                Self: ::planus::WriteAsOffset<DvInt>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<i64, i64>> ::planus::WriteAs<::planus::Offset<DvInt>>
            for DvIntBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvInt>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvInt> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<i64, i64>>
            ::planus::WriteAsOptional<::planus::Offset<DvInt>> for DvIntBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvInt>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvInt>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsDefault<i64, i64>> ::planus::WriteAsOffset<DvInt>
            for DvIntBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvInt> {
                let (v0,) = &self.0;
                DvInt::create(builder, v0)
            }
        }

        /// Reference to a deserialized [DvInt].
        #[derive(Copy, Clone)]
        pub struct DvIntRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvIntRef<'a> {
            /// Getter for the [`v` field](DvInt#structfield.v).
            #[inline]
            pub fn v(&self) -> ::planus::Result<i64> {
                ::core::result::Result::Ok(self.0.access(0, "DvInt", "v")?.unwrap_or(0))
            }
        }

        impl<'a> ::core::fmt::Debug for DvIntRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvIntRef");
                f.field("v", &self.v());
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvIntRef<'a>> for DvInt {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DvIntRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    v: ::core::convert::TryInto::try_into(value.v()?)?,
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvIntRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvIntRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[DvIntRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvInt>> for DvInt {
            type Value = ::planus::Offset<DvInt>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvInt>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvIntRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvIntRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvBigInt` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvBigInt` in the file `crates/quoin-ext-proto/schema/ext.fbs:39`
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
        pub struct DvBigInt {
            /// The field `v` in the table `DvBigInt`
            pub v: ::core::option::Option<::planus::alloc::string::String>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvBigInt {
            fn default() -> Self {
                Self {
                    v: ::core::default::Default::default(),
                }
            }
        }

        impl DvBigInt {
            /// Creates a [DvBigIntBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvBigIntBuilder<()> {
                DvBigIntBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_v: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            ) -> ::planus::Offset<Self> {
                let prepared_v = field_v.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_v.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_v) = prepared_v {
                            object_writer.write::<_, _, 4>(&prepared_v);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvBigInt>> for DvBigInt {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBigInt> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvBigInt>> for DvBigInt {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvBigInt>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvBigInt> for DvBigInt {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBigInt> {
                DvBigInt::create(builder, &self.v)
            }
        }

        /// Builder for serializing an instance of the [DvBigInt] type.
        ///
        /// Can be created using the [DvBigInt::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvBigIntBuilder<State>(State);

        impl DvBigIntBuilder<()> {
            /// Setter for the [`v` field](DvBigInt#structfield.v).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v<T0>(self, value: T0) -> DvBigIntBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                DvBigIntBuilder((value,))
            }

            /// Sets the [`v` field](DvBigInt#structfield.v) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v_as_null(self) -> DvBigIntBuilder<((),)> {
                self.v(())
            }
        }

        impl<T0> DvBigIntBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvBigInt].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBigInt>
            where
                Self: ::planus::WriteAsOffset<DvBigInt>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAs<::planus::Offset<DvBigInt>> for DvBigIntBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvBigInt>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBigInt> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOptional<::planus::Offset<DvBigInt>> for DvBigIntBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvBigInt>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvBigInt>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOffset<DvBigInt> for DvBigIntBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBigInt> {
                let (v0,) = &self.0;
                DvBigInt::create(builder, v0)
            }
        }

        /// Reference to a deserialized [DvBigInt].
        #[derive(Copy, Clone)]
        pub struct DvBigIntRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvBigIntRef<'a> {
            /// Getter for the [`v` field](DvBigInt#structfield.v).
            #[inline]
            pub fn v(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(0, "DvBigInt", "v")
            }
        }

        impl<'a> ::core::fmt::Debug for DvBigIntRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvBigIntRef");
                if let ::core::option::Option::Some(field_v) = self.v().transpose() {
                    f.field("v", &field_v);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvBigIntRef<'a>> for DvBigInt {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DvBigIntRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    v: value.v()?.map(::core::convert::Into::into),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvBigIntRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvBigIntRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[DvBigIntRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvBigInt>> for DvBigInt {
            type Value = ::planus::Offset<DvBigInt>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvBigInt>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvBigIntRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvBigIntRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvFloat` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvFloat` in the file `crates/quoin-ext-proto/schema/ext.fbs:40`
        #[derive(Clone, Debug, PartialEq, PartialOrd, ::serde::Serialize, ::serde::Deserialize)]
        pub struct DvFloat {
            /// The field `v` in the table `DvFloat`
            pub v: f64,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvFloat {
            fn default() -> Self {
                Self { v: 0.0 }
            }
        }

        impl DvFloat {
            /// Creates a [DvFloatBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvFloatBuilder<()> {
                DvFloatBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_v: impl ::planus::WriteAsDefault<f64, f64>,
            ) -> ::planus::Offset<Self> {
                let prepared_v = field_v.prepare(builder, &0.0);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_v.is_some() {
                    table_writer.write_entry::<f64>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_v) = prepared_v {
                            object_writer.write::<_, _, 8>(&prepared_v);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvFloat>> for DvFloat {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvFloat> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvFloat>> for DvFloat {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvFloat>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvFloat> for DvFloat {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvFloat> {
                DvFloat::create(builder, self.v)
            }
        }

        /// Builder for serializing an instance of the [DvFloat] type.
        ///
        /// Can be created using the [DvFloat::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvFloatBuilder<State>(State);

        impl DvFloatBuilder<()> {
            /// Setter for the [`v` field](DvFloat#structfield.v).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v<T0>(self, value: T0) -> DvFloatBuilder<(T0,)>
            where
                T0: ::planus::WriteAsDefault<f64, f64>,
            {
                DvFloatBuilder((value,))
            }

            /// Sets the [`v` field](DvFloat#structfield.v) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v_as_default(self) -> DvFloatBuilder<(::planus::DefaultValue,)> {
                self.v(::planus::DefaultValue)
            }
        }

        impl<T0> DvFloatBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvFloat].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvFloat>
            where
                Self: ::planus::WriteAsOffset<DvFloat>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<f64, f64>> ::planus::WriteAs<::planus::Offset<DvFloat>>
            for DvFloatBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvFloat>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvFloat> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<f64, f64>>
            ::planus::WriteAsOptional<::planus::Offset<DvFloat>> for DvFloatBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvFloat>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvFloat>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsDefault<f64, f64>> ::planus::WriteAsOffset<DvFloat>
            for DvFloatBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvFloat> {
                let (v0,) = &self.0;
                DvFloat::create(builder, v0)
            }
        }

        /// Reference to a deserialized [DvFloat].
        #[derive(Copy, Clone)]
        pub struct DvFloatRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvFloatRef<'a> {
            /// Getter for the [`v` field](DvFloat#structfield.v).
            #[inline]
            pub fn v(&self) -> ::planus::Result<f64> {
                ::core::result::Result::Ok(self.0.access(0, "DvFloat", "v")?.unwrap_or(0.0))
            }
        }

        impl<'a> ::core::fmt::Debug for DvFloatRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvFloatRef");
                f.field("v", &self.v());
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvFloatRef<'a>> for DvFloat {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DvFloatRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    v: ::core::convert::TryInto::try_into(value.v()?)?,
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvFloatRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvFloatRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[DvFloatRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvFloat>> for DvFloat {
            type Value = ::planus::Offset<DvFloat>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvFloat>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvFloatRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvFloatRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvDecimal` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvDecimal` in the file `crates/quoin-ext-proto/schema/ext.fbs:41`
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
        pub struct DvDecimal {
            /// The field `v` in the table `DvDecimal`
            pub v: ::core::option::Option<::planus::alloc::string::String>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvDecimal {
            fn default() -> Self {
                Self {
                    v: ::core::default::Default::default(),
                }
            }
        }

        impl DvDecimal {
            /// Creates a [DvDecimalBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvDecimalBuilder<()> {
                DvDecimalBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_v: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            ) -> ::planus::Offset<Self> {
                let prepared_v = field_v.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_v.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_v) = prepared_v {
                            object_writer.write::<_, _, 4>(&prepared_v);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvDecimal>> for DvDecimal {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvDecimal> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvDecimal>> for DvDecimal {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvDecimal>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvDecimal> for DvDecimal {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvDecimal> {
                DvDecimal::create(builder, &self.v)
            }
        }

        /// Builder for serializing an instance of the [DvDecimal] type.
        ///
        /// Can be created using the [DvDecimal::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvDecimalBuilder<State>(State);

        impl DvDecimalBuilder<()> {
            /// Setter for the [`v` field](DvDecimal#structfield.v).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v<T0>(self, value: T0) -> DvDecimalBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                DvDecimalBuilder((value,))
            }

            /// Sets the [`v` field](DvDecimal#structfield.v) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v_as_null(self) -> DvDecimalBuilder<((),)> {
                self.v(())
            }
        }

        impl<T0> DvDecimalBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvDecimal].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvDecimal>
            where
                Self: ::planus::WriteAsOffset<DvDecimal>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAs<::planus::Offset<DvDecimal>> for DvDecimalBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvDecimal>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvDecimal> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOptional<::planus::Offset<DvDecimal>> for DvDecimalBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvDecimal>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvDecimal>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOffset<DvDecimal> for DvDecimalBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvDecimal> {
                let (v0,) = &self.0;
                DvDecimal::create(builder, v0)
            }
        }

        /// Reference to a deserialized [DvDecimal].
        #[derive(Copy, Clone)]
        pub struct DvDecimalRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvDecimalRef<'a> {
            /// Getter for the [`v` field](DvDecimal#structfield.v).
            #[inline]
            pub fn v(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(0, "DvDecimal", "v")
            }
        }

        impl<'a> ::core::fmt::Debug for DvDecimalRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvDecimalRef");
                if let ::core::option::Option::Some(field_v) = self.v().transpose() {
                    f.field("v", &field_v);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvDecimalRef<'a>> for DvDecimal {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DvDecimalRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    v: value.v()?.map(::core::convert::Into::into),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvDecimalRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvDecimalRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[DvDecimalRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvDecimal>> for DvDecimal {
            type Value = ::planus::Offset<DvDecimal>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvDecimal>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvDecimalRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvDecimalRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvStr` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvStr` in the file `crates/quoin-ext-proto/schema/ext.fbs:42`
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
        pub struct DvStr {
            /// The field `v` in the table `DvStr`
            pub v: ::core::option::Option<::planus::alloc::string::String>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvStr {
            fn default() -> Self {
                Self {
                    v: ::core::default::Default::default(),
                }
            }
        }

        impl DvStr {
            /// Creates a [DvStrBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvStrBuilder<()> {
                DvStrBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_v: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            ) -> ::planus::Offset<Self> {
                let prepared_v = field_v.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_v.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_v) = prepared_v {
                            object_writer.write::<_, _, 4>(&prepared_v);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvStr>> for DvStr {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvStr> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvStr>> for DvStr {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvStr>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvStr> for DvStr {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvStr> {
                DvStr::create(builder, &self.v)
            }
        }

        /// Builder for serializing an instance of the [DvStr] type.
        ///
        /// Can be created using the [DvStr::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvStrBuilder<State>(State);

        impl DvStrBuilder<()> {
            /// Setter for the [`v` field](DvStr#structfield.v).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v<T0>(self, value: T0) -> DvStrBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                DvStrBuilder((value,))
            }

            /// Sets the [`v` field](DvStr#structfield.v) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v_as_null(self) -> DvStrBuilder<((),)> {
                self.v(())
            }
        }

        impl<T0> DvStrBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvStr].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvStr>
            where
                Self: ::planus::WriteAsOffset<DvStr>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAs<::planus::Offset<DvStr>> for DvStrBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvStr>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvStr> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOptional<::planus::Offset<DvStr>> for DvStrBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvStr>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvStr>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>>
            ::planus::WriteAsOffset<DvStr> for DvStrBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvStr> {
                let (v0,) = &self.0;
                DvStr::create(builder, v0)
            }
        }

        /// Reference to a deserialized [DvStr].
        #[derive(Copy, Clone)]
        pub struct DvStrRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvStrRef<'a> {
            /// Getter for the [`v` field](DvStr#structfield.v).
            #[inline]
            pub fn v(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(0, "DvStr", "v")
            }
        }

        impl<'a> ::core::fmt::Debug for DvStrRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvStrRef");
                if let ::core::option::Option::Some(field_v) = self.v().transpose() {
                    f.field("v", &field_v);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvStrRef<'a>> for DvStr {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DvStrRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    v: value.v()?.map(::core::convert::Into::into),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvStrRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvStrRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[DvStrRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvStr>> for DvStr {
            type Value = ::planus::Offset<DvStr>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvStr>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvStrRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvStrRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvBytes` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvBytes` in the file `crates/quoin-ext-proto/schema/ext.fbs:43`
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
        pub struct DvBytes {
            /// The field `v` in the table `DvBytes`
            pub v: ::core::option::Option<::planus::alloc::vec::Vec<u8>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvBytes {
            fn default() -> Self {
                Self {
                    v: ::core::default::Default::default(),
                }
            }
        }

        impl DvBytes {
            /// Creates a [DvBytesBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvBytesBuilder<()> {
                DvBytesBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_v: impl ::planus::WriteAsOptional<::planus::Offset<[u8]>>,
            ) -> ::planus::Offset<Self> {
                let prepared_v = field_v.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_v.is_some() {
                    table_writer.write_entry::<::planus::Offset<[u8]>>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_v) = prepared_v {
                            object_writer.write::<_, _, 4>(&prepared_v);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvBytes>> for DvBytes {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBytes> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvBytes>> for DvBytes {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvBytes>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvBytes> for DvBytes {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBytes> {
                DvBytes::create(builder, &self.v)
            }
        }

        /// Builder for serializing an instance of the [DvBytes] type.
        ///
        /// Can be created using the [DvBytes::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvBytesBuilder<State>(State);

        impl DvBytesBuilder<()> {
            /// Setter for the [`v` field](DvBytes#structfield.v).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v<T0>(self, value: T0) -> DvBytesBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<[u8]>>,
            {
                DvBytesBuilder((value,))
            }

            /// Sets the [`v` field](DvBytes#structfield.v) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v_as_null(self) -> DvBytesBuilder<((),)> {
                self.v(())
            }
        }

        impl<T0> DvBytesBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvBytes].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBytes>
            where
                Self: ::planus::WriteAsOffset<DvBytes>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[u8]>>>
            ::planus::WriteAs<::planus::Offset<DvBytes>> for DvBytesBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvBytes>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBytes> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[u8]>>>
            ::planus::WriteAsOptional<::planus::Offset<DvBytes>> for DvBytesBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvBytes>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvBytes>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[u8]>>> ::planus::WriteAsOffset<DvBytes>
            for DvBytesBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvBytes> {
                let (v0,) = &self.0;
                DvBytes::create(builder, v0)
            }
        }

        /// Reference to a deserialized [DvBytes].
        #[derive(Copy, Clone)]
        pub struct DvBytesRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvBytesRef<'a> {
            /// Getter for the [`v` field](DvBytes#structfield.v).
            #[inline]
            pub fn v(&self) -> ::planus::Result<::core::option::Option<&'a [u8]>> {
                self.0.access(0, "DvBytes", "v")
            }
        }

        impl<'a> ::core::fmt::Debug for DvBytesRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvBytesRef");
                if let ::core::option::Option::Some(field_v) = self.v().transpose() {
                    f.field("v", &field_v);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvBytesRef<'a>> for DvBytes {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DvBytesRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    v: value.v()?.map(|v| v.to_vec()),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvBytesRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvBytesRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[DvBytesRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvBytes>> for DvBytes {
            type Value = ::planus::Offset<DvBytes>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvBytes>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvBytesRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvBytesRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvList` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvList` in the file `crates/quoin-ext-proto/schema/ext.fbs:44`
        #[derive(Clone, Debug, PartialEq, PartialOrd, ::serde::Serialize, ::serde::Deserialize)]
        pub struct DvList {
            /// The field `items` in the table `DvList`
            pub items: ::core::option::Option<::planus::alloc::vec::Vec<self::DataValueBox>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvList {
            fn default() -> Self {
                Self {
                    items: ::core::default::Default::default(),
                }
            }
        }

        impl DvList {
            /// Creates a [DvListBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvListBuilder<()> {
                DvListBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_items: impl ::planus::WriteAsOptional<
                    ::planus::Offset<[::planus::Offset<self::DataValueBox>]>,
                >,
            ) -> ::planus::Offset<Self> {
                let prepared_items = field_items.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_items.is_some() {
                    table_writer
                        .write_entry::<::planus::Offset<[::planus::Offset<self::DataValueBox>]>>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_items) = prepared_items {
                            object_writer.write::<_, _, 4>(&prepared_items);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvList>> for DvList {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvList> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvList>> for DvList {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvList>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvList> for DvList {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvList> {
                DvList::create(builder, &self.items)
            }
        }

        /// Builder for serializing an instance of the [DvList] type.
        ///
        /// Can be created using the [DvList::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvListBuilder<State>(State);

        impl DvListBuilder<()> {
            /// Setter for the [`items` field](DvList#structfield.items).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn items<T0>(self, value: T0) -> DvListBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<
                        ::planus::Offset<[::planus::Offset<self::DataValueBox>]>,
                    >,
            {
                DvListBuilder((value,))
            }

            /// Sets the [`items` field](DvList#structfield.items) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn items_as_null(self) -> DvListBuilder<((),)> {
                self.items(())
            }
        }

        impl<T0> DvListBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvList].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvList>
            where
                Self: ::planus::WriteAsOffset<DvList>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::DataValueBox>]>>,
        > ::planus::WriteAs<::planus::Offset<DvList>> for DvListBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvList>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvList> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::DataValueBox>]>>,
        > ::planus::WriteAsOptional<::planus::Offset<DvList>> for DvListBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvList>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvList>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::DataValueBox>]>>,
        > ::planus::WriteAsOffset<DvList> for DvListBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvList> {
                let (v0,) = &self.0;
                DvList::create(builder, v0)
            }
        }

        /// Reference to a deserialized [DvList].
        #[derive(Copy, Clone)]
        pub struct DvListRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvListRef<'a> {
            /// Getter for the [`items` field](DvList#structfield.items).
            #[inline]
            pub fn items(
                &self,
            ) -> ::planus::Result<
                ::core::option::Option<
                    ::planus::Vector<'a, ::planus::Result<self::DataValueBoxRef<'a>>>,
                >,
            > {
                self.0.access(0, "DvList", "items")
            }
        }

        impl<'a> ::core::fmt::Debug for DvListRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvListRef");
                if let ::core::option::Option::Some(field_items) = self.items().transpose() {
                    f.field("items", &field_items);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvListRef<'a>> for DvList {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DvListRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    items: if let ::core::option::Option::Some(items) = value.items()? {
                        ::core::option::Option::Some(items.to_vec_result()?)
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvListRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvListRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[DvListRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvList>> for DvList {
            type Value = ::planus::Offset<DvList>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvList>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvListRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvListRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvEntry` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvEntry` in the file `crates/quoin-ext-proto/schema/ext.fbs:45`
        #[derive(Clone, Debug, PartialEq, PartialOrd, ::serde::Serialize, ::serde::Deserialize)]
        pub struct DvEntry {
            /// The field `key` in the table `DvEntry`
            pub key: ::core::option::Option<::planus::alloc::string::String>,
            /// The field `value` in the table `DvEntry`
            pub value: ::core::option::Option<::planus::alloc::boxed::Box<self::DataValueBox>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvEntry {
            fn default() -> Self {
                Self {
                    key: ::core::default::Default::default(),
                    value: ::core::default::Default::default(),
                }
            }
        }

        impl DvEntry {
            /// Creates a [DvEntryBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvEntryBuilder<()> {
                DvEntryBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_key: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
                field_value: impl ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
            ) -> ::planus::Offset<Self> {
                let prepared_key = field_key.prepare(builder);
                let prepared_value = field_value.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<8> =
                    ::core::default::Default::default();
                if prepared_key.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(0);
                }
                if prepared_value.is_some() {
                    table_writer.write_entry::<::planus::Offset<self::DataValueBox>>(1);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_key) = prepared_key {
                            object_writer.write::<_, _, 4>(&prepared_key);
                        }
                        if let ::core::option::Option::Some(prepared_value) = prepared_value {
                            object_writer.write::<_, _, 4>(&prepared_value);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvEntry>> for DvEntry {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvEntry> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvEntry>> for DvEntry {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvEntry>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvEntry> for DvEntry {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvEntry> {
                DvEntry::create(builder, &self.key, &self.value)
            }
        }

        /// Builder for serializing an instance of the [DvEntry] type.
        ///
        /// Can be created using the [DvEntry::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvEntryBuilder<State>(State);

        impl DvEntryBuilder<()> {
            /// Setter for the [`key` field](DvEntry#structfield.key).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn key<T0>(self, value: T0) -> DvEntryBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                DvEntryBuilder((value,))
            }

            /// Sets the [`key` field](DvEntry#structfield.key) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn key_as_null(self) -> DvEntryBuilder<((),)> {
                self.key(())
            }
        }

        impl<T0> DvEntryBuilder<(T0,)> {
            /// Setter for the [`value` field](DvEntry#structfield.value).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn value<T1>(self, value: T1) -> DvEntryBuilder<(T0, T1)>
            where
                T1: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
            {
                let (v0,) = self.0;
                DvEntryBuilder((v0, value))
            }

            /// Sets the [`value` field](DvEntry#structfield.value) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn value_as_null(self) -> DvEntryBuilder<(T0, ())> {
                self.value(())
            }
        }

        impl<T0, T1> DvEntryBuilder<(T0, T1)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvEntry].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvEntry>
            where
                Self: ::planus::WriteAsOffset<DvEntry>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
        > ::planus::WriteAs<::planus::Offset<DvEntry>> for DvEntryBuilder<(T0, T1)>
        {
            type Prepared = ::planus::Offset<DvEntry>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvEntry> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
        > ::planus::WriteAsOptional<::planus::Offset<DvEntry>> for DvEntryBuilder<(T0, T1)>
        {
            type Prepared = ::planus::Offset<DvEntry>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvEntry>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
        > ::planus::WriteAsOffset<DvEntry> for DvEntryBuilder<(T0, T1)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvEntry> {
                let (v0, v1) = &self.0;
                DvEntry::create(builder, v0, v1)
            }
        }

        /// Reference to a deserialized [DvEntry].
        #[derive(Copy, Clone)]
        pub struct DvEntryRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvEntryRef<'a> {
            /// Getter for the [`key` field](DvEntry#structfield.key).
            #[inline]
            pub fn key(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(0, "DvEntry", "key")
            }

            /// Getter for the [`value` field](DvEntry#structfield.value).
            #[inline]
            pub fn value(
                &self,
            ) -> ::planus::Result<::core::option::Option<self::DataValueBoxRef<'a>>> {
                self.0.access(1, "DvEntry", "value")
            }
        }

        impl<'a> ::core::fmt::Debug for DvEntryRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvEntryRef");
                if let ::core::option::Option::Some(field_key) = self.key().transpose() {
                    f.field("key", &field_key);
                }
                if let ::core::option::Option::Some(field_value) = self.value().transpose() {
                    f.field("value", &field_value);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvEntryRef<'a>> for DvEntry {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DvEntryRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    key: value.key()?.map(::core::convert::Into::into),
                    value: if let ::core::option::Option::Some(value) = value.value()? {
                        ::core::option::Option::Some(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryInto::try_into(value)?,
                        ))
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvEntryRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvEntryRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[DvEntryRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvEntry>> for DvEntry {
            type Value = ::planus::Offset<DvEntry>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvEntry>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvEntryRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvEntryRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `DvMap` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DvMap` in the file `crates/quoin-ext-proto/schema/ext.fbs:46`
        #[derive(Clone, Debug, PartialEq, PartialOrd, ::serde::Serialize, ::serde::Deserialize)]
        pub struct DvMap {
            /// The field `entries` in the table `DvMap`
            pub entries: ::core::option::Option<::planus::alloc::vec::Vec<self::DvEntry>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DvMap {
            fn default() -> Self {
                Self {
                    entries: ::core::default::Default::default(),
                }
            }
        }

        impl DvMap {
            /// Creates a [DvMapBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DvMapBuilder<()> {
                DvMapBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_entries: impl ::planus::WriteAsOptional<
                    ::planus::Offset<[::planus::Offset<self::DvEntry>]>,
                >,
            ) -> ::planus::Offset<Self> {
                let prepared_entries = field_entries.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_entries.is_some() {
                    table_writer
                        .write_entry::<::planus::Offset<[::planus::Offset<self::DvEntry>]>>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_entries) = prepared_entries {
                            object_writer.write::<_, _, 4>(&prepared_entries);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DvMap>> for DvMap {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvMap> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DvMap>> for DvMap {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvMap>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DvMap> for DvMap {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvMap> {
                DvMap::create(builder, &self.entries)
            }
        }

        /// Builder for serializing an instance of the [DvMap] type.
        ///
        /// Can be created using the [DvMap::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DvMapBuilder<State>(State);

        impl DvMapBuilder<()> {
            /// Setter for the [`entries` field](DvMap#structfield.entries).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn entries<T0>(self, value: T0) -> DvMapBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::DvEntry>]>>,
            {
                DvMapBuilder((value,))
            }

            /// Sets the [`entries` field](DvMap#structfield.entries) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn entries_as_null(self) -> DvMapBuilder<((),)> {
                self.entries(())
            }
        }

        impl<T0> DvMapBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DvMap].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvMap>
            where
                Self: ::planus::WriteAsOffset<DvMap>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::DvEntry>]>>>
            ::planus::WriteAs<::planus::Offset<DvMap>> for DvMapBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvMap>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvMap> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::DvEntry>]>>>
            ::planus::WriteAsOptional<::planus::Offset<DvMap>> for DvMapBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DvMap>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DvMap>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::DvEntry>]>>>
            ::planus::WriteAsOffset<DvMap> for DvMapBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DvMap> {
                let (v0,) = &self.0;
                DvMap::create(builder, v0)
            }
        }

        /// Reference to a deserialized [DvMap].
        #[derive(Copy, Clone)]
        pub struct DvMapRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DvMapRef<'a> {
            /// Getter for the [`entries` field](DvMap#structfield.entries).
            #[inline]
            pub fn entries(
                &self,
            ) -> ::planus::Result<
                ::core::option::Option<
                    ::planus::Vector<'a, ::planus::Result<self::DvEntryRef<'a>>>,
                >,
            > {
                self.0.access(0, "DvMap", "entries")
            }
        }

        impl<'a> ::core::fmt::Debug for DvMapRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DvMapRef");
                if let ::core::option::Option::Some(field_entries) = self.entries().transpose() {
                    f.field("entries", &field_entries);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DvMapRef<'a>> for DvMap {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DvMapRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    entries: if let ::core::option::Option::Some(entries) = value.entries()? {
                        ::core::option::Option::Some(entries.to_vec_result()?)
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DvMapRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DvMapRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location("[DvMapRef]", "get", buffer.offset_from_start)
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DvMap>> for DvMap {
            type Value = ::planus::Offset<DvMap>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DvMap>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DvMapRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DvMapRef]", "read_as_root", 0)
                })
            }
        }

        /// The union `DataValueKind` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Union `DataValueKind` in the file `crates/quoin-ext-proto/schema/ext.fbs:48`
        #[derive(Clone, Debug, PartialEq, PartialOrd, ::serde::Serialize, ::serde::Deserialize)]
        pub enum DataValueKind {
            /// The variant of type `DvNull` in the union `DataValueKind`
            DvNull(::planus::alloc::boxed::Box<self::DvNull>),

            /// The variant of type `DvBool` in the union `DataValueKind`
            DvBool(::planus::alloc::boxed::Box<self::DvBool>),

            /// The variant of type `DvInt` in the union `DataValueKind`
            DvInt(::planus::alloc::boxed::Box<self::DvInt>),

            /// The variant of type `DvBigInt` in the union `DataValueKind`
            DvBigInt(::planus::alloc::boxed::Box<self::DvBigInt>),

            /// The variant of type `DvFloat` in the union `DataValueKind`
            DvFloat(::planus::alloc::boxed::Box<self::DvFloat>),

            /// The variant of type `DvDecimal` in the union `DataValueKind`
            DvDecimal(::planus::alloc::boxed::Box<self::DvDecimal>),

            /// The variant of type `DvStr` in the union `DataValueKind`
            DvStr(::planus::alloc::boxed::Box<self::DvStr>),

            /// The variant of type `DvBytes` in the union `DataValueKind`
            DvBytes(::planus::alloc::boxed::Box<self::DvBytes>),

            /// The variant of type `DvList` in the union `DataValueKind`
            DvList(::planus::alloc::boxed::Box<self::DvList>),

            /// The variant of type `DvMap` in the union `DataValueKind`
            DvMap(::planus::alloc::boxed::Box<self::DvMap>),
        }

        impl DataValueKind {
            /// Creates a [DataValueKindBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DataValueKindBuilder<::planus::Uninitialized> {
                DataValueKindBuilder(::planus::Uninitialized)
            }

            #[inline]
            pub fn create_dv_null(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::DvNull>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(1, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_dv_bool(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::DvBool>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(2, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_dv_int(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::DvInt>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(3, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_dv_big_int(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::DvBigInt>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(4, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_dv_float(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::DvFloat>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(5, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_dv_decimal(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::DvDecimal>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(6, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_dv_str(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::DvStr>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(7, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_dv_bytes(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::DvBytes>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(8, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_dv_list(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::DvList>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(9, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_dv_map(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::DvMap>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(10, value.prepare(builder).downcast())
            }
        }

        impl ::planus::WriteAsUnion<DataValueKind> for DataValueKind {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Self> {
                match self {
                    Self::DvNull(value) => Self::create_dv_null(builder, value),
                    Self::DvBool(value) => Self::create_dv_bool(builder, value),
                    Self::DvInt(value) => Self::create_dv_int(builder, value),
                    Self::DvBigInt(value) => Self::create_dv_big_int(builder, value),
                    Self::DvFloat(value) => Self::create_dv_float(builder, value),
                    Self::DvDecimal(value) => Self::create_dv_decimal(builder, value),
                    Self::DvStr(value) => Self::create_dv_str(builder, value),
                    Self::DvBytes(value) => Self::create_dv_bytes(builder, value),
                    Self::DvList(value) => Self::create_dv_list(builder, value),
                    Self::DvMap(value) => Self::create_dv_map(builder, value),
                }
            }
        }

        impl ::planus::WriteAsOptionalUnion<DataValueKind> for DataValueKind {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Self>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }

        /// Builder for serializing an instance of the [DataValueKind] type.
        ///
        /// Can be created using the [DataValueKind::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DataValueKindBuilder<T>(T);

        impl DataValueKindBuilder<::planus::Uninitialized> {
            /// Creates an instance of the [`DvNull` variant](DataValueKind#variant.DvNull).
            #[inline]
            pub fn dv_null<T>(self, value: T) -> DataValueKindBuilder<::planus::Initialized<1, T>>
            where
                T: ::planus::WriteAsOffset<self::DvNull>,
            {
                DataValueKindBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`DvBool` variant](DataValueKind#variant.DvBool).
            #[inline]
            pub fn dv_bool<T>(self, value: T) -> DataValueKindBuilder<::planus::Initialized<2, T>>
            where
                T: ::planus::WriteAsOffset<self::DvBool>,
            {
                DataValueKindBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`DvInt` variant](DataValueKind#variant.DvInt).
            #[inline]
            pub fn dv_int<T>(self, value: T) -> DataValueKindBuilder<::planus::Initialized<3, T>>
            where
                T: ::planus::WriteAsOffset<self::DvInt>,
            {
                DataValueKindBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`DvBigInt` variant](DataValueKind#variant.DvBigInt).
            #[inline]
            pub fn dv_big_int<T>(
                self,
                value: T,
            ) -> DataValueKindBuilder<::planus::Initialized<4, T>>
            where
                T: ::planus::WriteAsOffset<self::DvBigInt>,
            {
                DataValueKindBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`DvFloat` variant](DataValueKind#variant.DvFloat).
            #[inline]
            pub fn dv_float<T>(self, value: T) -> DataValueKindBuilder<::planus::Initialized<5, T>>
            where
                T: ::planus::WriteAsOffset<self::DvFloat>,
            {
                DataValueKindBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`DvDecimal` variant](DataValueKind#variant.DvDecimal).
            #[inline]
            pub fn dv_decimal<T>(
                self,
                value: T,
            ) -> DataValueKindBuilder<::planus::Initialized<6, T>>
            where
                T: ::planus::WriteAsOffset<self::DvDecimal>,
            {
                DataValueKindBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`DvStr` variant](DataValueKind#variant.DvStr).
            #[inline]
            pub fn dv_str<T>(self, value: T) -> DataValueKindBuilder<::planus::Initialized<7, T>>
            where
                T: ::planus::WriteAsOffset<self::DvStr>,
            {
                DataValueKindBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`DvBytes` variant](DataValueKind#variant.DvBytes).
            #[inline]
            pub fn dv_bytes<T>(self, value: T) -> DataValueKindBuilder<::planus::Initialized<8, T>>
            where
                T: ::planus::WriteAsOffset<self::DvBytes>,
            {
                DataValueKindBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`DvList` variant](DataValueKind#variant.DvList).
            #[inline]
            pub fn dv_list<T>(self, value: T) -> DataValueKindBuilder<::planus::Initialized<9, T>>
            where
                T: ::planus::WriteAsOffset<self::DvList>,
            {
                DataValueKindBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`DvMap` variant](DataValueKind#variant.DvMap).
            #[inline]
            pub fn dv_map<T>(self, value: T) -> DataValueKindBuilder<::planus::Initialized<10, T>>
            where
                T: ::planus::WriteAsOffset<self::DvMap>,
            {
                DataValueKindBuilder(::planus::Initialized(value))
            }
        }

        impl<const N: u8, T> DataValueKindBuilder<::planus::Initialized<N, T>> {
            /// Finish writing the builder to get an [UnionOffset](::planus::UnionOffset) to a serialized [DataValueKind].
            #[inline]
            pub fn finish(
                self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind>
            where
                Self: ::planus::WriteAsUnion<DataValueKind>,
            {
                ::planus::WriteAsUnion::prepare(&self, builder)
            }
        }

        impl<T> ::planus::WriteAsUnion<DataValueKind> for DataValueKindBuilder<::planus::Initialized<1, T>>
        where
            T: ::planus::WriteAsOffset<self::DvNull>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind> {
                ::planus::UnionOffset::new(1, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<DataValueKind>
            for DataValueKindBuilder<::planus::Initialized<1, T>>
        where
            T: ::planus::WriteAsOffset<self::DvNull>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<DataValueKind>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<DataValueKind> for DataValueKindBuilder<::planus::Initialized<2, T>>
        where
            T: ::planus::WriteAsOffset<self::DvBool>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind> {
                ::planus::UnionOffset::new(2, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<DataValueKind>
            for DataValueKindBuilder<::planus::Initialized<2, T>>
        where
            T: ::planus::WriteAsOffset<self::DvBool>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<DataValueKind>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<DataValueKind> for DataValueKindBuilder<::planus::Initialized<3, T>>
        where
            T: ::planus::WriteAsOffset<self::DvInt>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind> {
                ::planus::UnionOffset::new(3, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<DataValueKind>
            for DataValueKindBuilder<::planus::Initialized<3, T>>
        where
            T: ::planus::WriteAsOffset<self::DvInt>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<DataValueKind>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<DataValueKind> for DataValueKindBuilder<::planus::Initialized<4, T>>
        where
            T: ::planus::WriteAsOffset<self::DvBigInt>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind> {
                ::planus::UnionOffset::new(4, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<DataValueKind>
            for DataValueKindBuilder<::planus::Initialized<4, T>>
        where
            T: ::planus::WriteAsOffset<self::DvBigInt>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<DataValueKind>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<DataValueKind> for DataValueKindBuilder<::planus::Initialized<5, T>>
        where
            T: ::planus::WriteAsOffset<self::DvFloat>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind> {
                ::planus::UnionOffset::new(5, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<DataValueKind>
            for DataValueKindBuilder<::planus::Initialized<5, T>>
        where
            T: ::planus::WriteAsOffset<self::DvFloat>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<DataValueKind>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<DataValueKind> for DataValueKindBuilder<::planus::Initialized<6, T>>
        where
            T: ::planus::WriteAsOffset<self::DvDecimal>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind> {
                ::planus::UnionOffset::new(6, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<DataValueKind>
            for DataValueKindBuilder<::planus::Initialized<6, T>>
        where
            T: ::planus::WriteAsOffset<self::DvDecimal>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<DataValueKind>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<DataValueKind> for DataValueKindBuilder<::planus::Initialized<7, T>>
        where
            T: ::planus::WriteAsOffset<self::DvStr>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind> {
                ::planus::UnionOffset::new(7, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<DataValueKind>
            for DataValueKindBuilder<::planus::Initialized<7, T>>
        where
            T: ::planus::WriteAsOffset<self::DvStr>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<DataValueKind>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<DataValueKind> for DataValueKindBuilder<::planus::Initialized<8, T>>
        where
            T: ::planus::WriteAsOffset<self::DvBytes>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind> {
                ::planus::UnionOffset::new(8, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<DataValueKind>
            for DataValueKindBuilder<::planus::Initialized<8, T>>
        where
            T: ::planus::WriteAsOffset<self::DvBytes>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<DataValueKind>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<DataValueKind> for DataValueKindBuilder<::planus::Initialized<9, T>>
        where
            T: ::planus::WriteAsOffset<self::DvList>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind> {
                ::planus::UnionOffset::new(9, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<DataValueKind>
            for DataValueKindBuilder<::planus::Initialized<9, T>>
        where
            T: ::planus::WriteAsOffset<self::DvList>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<DataValueKind>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<DataValueKind> for DataValueKindBuilder<::planus::Initialized<10, T>>
        where
            T: ::planus::WriteAsOffset<self::DvMap>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::UnionOffset<DataValueKind> {
                ::planus::UnionOffset::new(10, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<DataValueKind>
            for DataValueKindBuilder<::planus::Initialized<10, T>>
        where
            T: ::planus::WriteAsOffset<self::DvMap>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<DataValueKind>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }

        /// Reference to a deserialized [DataValueKind].
        #[derive(Copy, Clone, Debug)]
        pub enum DataValueKindRef<'a> {
            DvNull(self::DvNullRef<'a>),
            DvBool(self::DvBoolRef<'a>),
            DvInt(self::DvIntRef<'a>),
            DvBigInt(self::DvBigIntRef<'a>),
            DvFloat(self::DvFloatRef<'a>),
            DvDecimal(self::DvDecimalRef<'a>),
            DvStr(self::DvStrRef<'a>),
            DvBytes(self::DvBytesRef<'a>),
            DvList(self::DvListRef<'a>),
            DvMap(self::DvMapRef<'a>),
        }

        impl<'a> ::core::convert::TryFrom<DataValueKindRef<'a>> for DataValueKind {
            type Error = ::planus::Error;

            fn try_from(value: DataValueKindRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(match value {
                    DataValueKindRef::DvNull(value) => {
                        Self::DvNull(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    DataValueKindRef::DvBool(value) => {
                        Self::DvBool(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    DataValueKindRef::DvInt(value) => {
                        Self::DvInt(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    DataValueKindRef::DvBigInt(value) => {
                        Self::DvBigInt(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    DataValueKindRef::DvFloat(value) => {
                        Self::DvFloat(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    DataValueKindRef::DvDecimal(value) => {
                        Self::DvDecimal(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    DataValueKindRef::DvStr(value) => {
                        Self::DvStr(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    DataValueKindRef::DvBytes(value) => {
                        Self::DvBytes(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    DataValueKindRef::DvList(value) => {
                        Self::DvList(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    DataValueKindRef::DvMap(value) => {
                        Self::DvMap(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }
                })
            }
        }

        impl<'a> ::planus::TableReadUnion<'a> for DataValueKindRef<'a> {
            fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                tag: u8,
                field_offset: usize,
            ) -> ::core::result::Result<Self, ::planus::errors::ErrorKind> {
                match tag {
                    1 => ::core::result::Result::Ok(Self::DvNull(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    2 => ::core::result::Result::Ok(Self::DvBool(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    3 => ::core::result::Result::Ok(Self::DvInt(::planus::TableRead::from_buffer(
                        buffer,
                        field_offset,
                    )?)),
                    4 => ::core::result::Result::Ok(Self::DvBigInt(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    5 => ::core::result::Result::Ok(Self::DvFloat(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    6 => ::core::result::Result::Ok(Self::DvDecimal(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    7 => ::core::result::Result::Ok(Self::DvStr(::planus::TableRead::from_buffer(
                        buffer,
                        field_offset,
                    )?)),
                    8 => ::core::result::Result::Ok(Self::DvBytes(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    9 => ::core::result::Result::Ok(Self::DvList(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    10 => ::core::result::Result::Ok(Self::DvMap(
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

        impl<'a> ::planus::VectorReadUnion<'a> for DataValueKindRef<'a> {
            const VECTOR_NAME: &'static str = "[DataValueKindRef]";
        }

        /// The table `DataValueBox` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `DataValueBox` in the file `crates/quoin-ext-proto/schema/ext.fbs:52`
        #[derive(Clone, Debug, PartialEq, PartialOrd, ::serde::Serialize, ::serde::Deserialize)]
        pub struct DataValueBox {
            /// The field `v` in the table `DataValueBox`
            pub v: ::core::option::Option<self::DataValueKind>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for DataValueBox {
            fn default() -> Self {
                Self {
                    v: ::core::default::Default::default(),
                }
            }
        }

        impl DataValueBox {
            /// Creates a [DataValueBoxBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> DataValueBoxBuilder<()> {
                DataValueBoxBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_v: impl ::planus::WriteAsOptionalUnion<self::DataValueKind>,
            ) -> ::planus::Offset<Self> {
                let prepared_v = field_v.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<8> =
                    ::core::default::Default::default();
                if prepared_v.is_some() {
                    table_writer.write_entry::<::planus::Offset<self::DataValueKind>>(1);
                }
                if prepared_v.is_some() {
                    table_writer.write_entry::<u8>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_v) = prepared_v {
                            object_writer.write::<_, _, 4>(&prepared_v.offset());
                        }
                        if let ::core::option::Option::Some(prepared_v) = prepared_v {
                            object_writer.write::<_, _, 1>(&prepared_v.tag());
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<DataValueBox>> for DataValueBox {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DataValueBox> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<DataValueBox>> for DataValueBox {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DataValueBox>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<DataValueBox> for DataValueBox {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DataValueBox> {
                DataValueBox::create(builder, &self.v)
            }
        }

        /// Builder for serializing an instance of the [DataValueBox] type.
        ///
        /// Can be created using the [DataValueBox::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct DataValueBoxBuilder<State>(State);

        impl DataValueBoxBuilder<()> {
            /// Setter for the [`v` field](DataValueBox#structfield.v).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v<T0>(self, value: T0) -> DataValueBoxBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptionalUnion<self::DataValueKind>,
            {
                DataValueBoxBuilder((value,))
            }

            /// Sets the [`v` field](DataValueBox#structfield.v) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn v_as_null(self) -> DataValueBoxBuilder<((),)> {
                self.v(())
            }
        }

        impl<T0> DataValueBoxBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [DataValueBox].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<DataValueBox>
            where
                Self: ::planus::WriteAsOffset<DataValueBox>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptionalUnion<self::DataValueKind>>
            ::planus::WriteAs<::planus::Offset<DataValueBox>> for DataValueBoxBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DataValueBox>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DataValueBox> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptionalUnion<self::DataValueKind>>
            ::planus::WriteAsOptional<::planus::Offset<DataValueBox>>
            for DataValueBoxBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<DataValueBox>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<DataValueBox>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptionalUnion<self::DataValueKind>>
            ::planus::WriteAsOffset<DataValueBox> for DataValueBoxBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<DataValueBox> {
                let (v0,) = &self.0;
                DataValueBox::create(builder, v0)
            }
        }

        /// Reference to a deserialized [DataValueBox].
        #[derive(Copy, Clone)]
        pub struct DataValueBoxRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> DataValueBoxRef<'a> {
            /// Getter for the [`v` field](DataValueBox#structfield.v).
            #[inline]
            pub fn v(
                &self,
            ) -> ::planus::Result<::core::option::Option<self::DataValueKindRef<'a>>> {
                self.0.access_union(0, "DataValueBox", "v")
            }
        }

        impl<'a> ::core::fmt::Debug for DataValueBoxRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("DataValueBoxRef");
                if let ::core::option::Option::Some(field_v) = self.v().transpose() {
                    f.field("v", &field_v);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<DataValueBoxRef<'a>> for DataValueBox {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: DataValueBoxRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    v: if let ::core::option::Option::Some(v) = value.v()? {
                        ::core::option::Option::Some(::core::convert::TryInto::try_into(v)?)
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for DataValueBoxRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for DataValueBoxRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[DataValueBoxRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<DataValueBox>> for DataValueBox {
            type Value = ::planus::Offset<DataValueBox>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<DataValueBox>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for DataValueBoxRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[DataValueBoxRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `Call` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `Call` in the file `crates/quoin-ext-proto/schema/ext.fbs:64`
        #[derive(Clone, Debug, PartialEq, PartialOrd, ::serde::Serialize, ::serde::Deserialize)]
        pub struct Call {
            /// The field `op` in the table `Call`
            pub op: ::core::option::Option<::planus::alloc::string::String>,
            /// The field `arg` in the table `Call`
            pub arg: ::core::option::Option<::planus::alloc::string::String>,
            /// The field `handles` in the table `Call`
            pub handles: ::core::option::Option<::planus::alloc::vec::Vec<u64>>,
            /// The field `resources` in the table `Call`
            pub resources: ::core::option::Option<::planus::alloc::vec::Vec<u64>>,
            /// The field `releases` in the table `Call`
            pub releases: ::core::option::Option<::planus::alloc::vec::Vec<u64>>,
            /// The field `arrays` in the table `Call`
            pub arrays: ::core::option::Option<::planus::alloc::vec::Vec<self::ArrowArray>>,
            /// The field `data` in the table `Call`
            pub data: ::core::option::Option<::planus::alloc::boxed::Box<self::DataValueBox>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for Call {
            fn default() -> Self {
                Self {
                    op: ::core::default::Default::default(),
                    arg: ::core::default::Default::default(),
                    handles: ::core::default::Default::default(),
                    resources: ::core::default::Default::default(),
                    releases: ::core::default::Default::default(),
                    arrays: ::core::default::Default::default(),
                    data: ::core::default::Default::default(),
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
                field_handles: impl ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
                field_resources: impl ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
                field_releases: impl ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
                field_arrays: impl ::planus::WriteAsOptional<
                    ::planus::Offset<[::planus::Offset<self::ArrowArray>]>,
                >,
                field_data: impl ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
            ) -> ::planus::Offset<Self> {
                let prepared_op = field_op.prepare(builder);
                let prepared_arg = field_arg.prepare(builder);
                let prepared_handles = field_handles.prepare(builder);
                let prepared_resources = field_resources.prepare(builder);
                let prepared_releases = field_releases.prepare(builder);
                let prepared_arrays = field_arrays.prepare(builder);
                let prepared_data = field_data.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<18> =
                    ::core::default::Default::default();
                if prepared_op.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(0);
                }
                if prepared_arg.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(1);
                }
                if prepared_handles.is_some() {
                    table_writer.write_entry::<::planus::Offset<[u64]>>(2);
                }
                if prepared_resources.is_some() {
                    table_writer.write_entry::<::planus::Offset<[u64]>>(3);
                }
                if prepared_releases.is_some() {
                    table_writer.write_entry::<::planus::Offset<[u64]>>(4);
                }
                if prepared_arrays.is_some() {
                    table_writer
                        .write_entry::<::planus::Offset<[::planus::Offset<self::ArrowArray>]>>(5);
                }
                if prepared_data.is_some() {
                    table_writer.write_entry::<::planus::Offset<self::DataValueBox>>(6);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_op) = prepared_op {
                            object_writer.write::<_, _, 4>(&prepared_op);
                        }
                        if let ::core::option::Option::Some(prepared_arg) = prepared_arg {
                            object_writer.write::<_, _, 4>(&prepared_arg);
                        }
                        if let ::core::option::Option::Some(prepared_handles) = prepared_handles {
                            object_writer.write::<_, _, 4>(&prepared_handles);
                        }
                        if let ::core::option::Option::Some(prepared_resources) = prepared_resources
                        {
                            object_writer.write::<_, _, 4>(&prepared_resources);
                        }
                        if let ::core::option::Option::Some(prepared_releases) = prepared_releases {
                            object_writer.write::<_, _, 4>(&prepared_releases);
                        }
                        if let ::core::option::Option::Some(prepared_arrays) = prepared_arrays {
                            object_writer.write::<_, _, 4>(&prepared_arrays);
                        }
                        if let ::core::option::Option::Some(prepared_data) = prepared_data {
                            object_writer.write::<_, _, 4>(&prepared_data);
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
                Call::create(
                    builder,
                    &self.op,
                    &self.arg,
                    &self.handles,
                    &self.resources,
                    &self.releases,
                    &self.arrays,
                    &self.data,
                )
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
            /// Setter for the [`handles` field](Call#structfield.handles).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handles<T2>(self, value: T2) -> CallBuilder<(T0, T1, T2)>
            where
                T2: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            {
                let (v0, v1) = self.0;
                CallBuilder((v0, v1, value))
            }

            /// Sets the [`handles` field](Call#structfield.handles) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handles_as_null(self) -> CallBuilder<(T0, T1, ())> {
                self.handles(())
            }
        }

        impl<T0, T1, T2> CallBuilder<(T0, T1, T2)> {
            /// Setter for the [`resources` field](Call#structfield.resources).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn resources<T3>(self, value: T3) -> CallBuilder<(T0, T1, T2, T3)>
            where
                T3: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            {
                let (v0, v1, v2) = self.0;
                CallBuilder((v0, v1, v2, value))
            }

            /// Sets the [`resources` field](Call#structfield.resources) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn resources_as_null(self) -> CallBuilder<(T0, T1, T2, ())> {
                self.resources(())
            }
        }

        impl<T0, T1, T2, T3> CallBuilder<(T0, T1, T2, T3)> {
            /// Setter for the [`releases` field](Call#structfield.releases).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn releases<T4>(self, value: T4) -> CallBuilder<(T0, T1, T2, T3, T4)>
            where
                T4: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            {
                let (v0, v1, v2, v3) = self.0;
                CallBuilder((v0, v1, v2, v3, value))
            }

            /// Sets the [`releases` field](Call#structfield.releases) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn releases_as_null(self) -> CallBuilder<(T0, T1, T2, T3, ())> {
                self.releases(())
            }
        }

        impl<T0, T1, T2, T3, T4> CallBuilder<(T0, T1, T2, T3, T4)> {
            /// Setter for the [`arrays` field](Call#structfield.arrays).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn arrays<T5>(self, value: T5) -> CallBuilder<(T0, T1, T2, T3, T4, T5)>
            where
                T5: ::planus::WriteAsOptional<
                        ::planus::Offset<[::planus::Offset<self::ArrowArray>]>,
                    >,
            {
                let (v0, v1, v2, v3, v4) = self.0;
                CallBuilder((v0, v1, v2, v3, v4, value))
            }

            /// Sets the [`arrays` field](Call#structfield.arrays) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn arrays_as_null(self) -> CallBuilder<(T0, T1, T2, T3, T4, ())> {
                self.arrays(())
            }
        }

        impl<T0, T1, T2, T3, T4, T5> CallBuilder<(T0, T1, T2, T3, T4, T5)> {
            /// Setter for the [`data` field](Call#structfield.data).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn data<T6>(self, value: T6) -> CallBuilder<(T0, T1, T2, T3, T4, T5, T6)>
            where
                T6: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
            {
                let (v0, v1, v2, v3, v4, v5) = self.0;
                CallBuilder((v0, v1, v2, v3, v4, v5, value))
            }

            /// Sets the [`data` field](Call#structfield.data) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn data_as_null(self) -> CallBuilder<(T0, T1, T2, T3, T4, T5, ())> {
                self.data(())
            }
        }

        impl<T0, T1, T2, T3, T4, T5, T6> CallBuilder<(T0, T1, T2, T3, T4, T5, T6)> {
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
            T2: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T3: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T4: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T5: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::ArrowArray>]>>,
            T6: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
        > ::planus::WriteAs<::planus::Offset<Call>> for CallBuilder<(T0, T1, T2, T3, T4, T5, T6)>
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
            T2: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T3: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T4: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T5: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::ArrowArray>]>>,
            T6: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
        > ::planus::WriteAsOptional<::planus::Offset<Call>>
            for CallBuilder<(T0, T1, T2, T3, T4, T5, T6)>
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
            T2: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T3: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T4: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T5: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::ArrowArray>]>>,
            T6: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
        > ::planus::WriteAsOffset<Call> for CallBuilder<(T0, T1, T2, T3, T4, T5, T6)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<Call> {
                let (v0, v1, v2, v3, v4, v5, v6) = &self.0;
                Call::create(builder, v0, v1, v2, v3, v4, v5, v6)
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

            /// Getter for the [`handles` field](Call#structfield.handles).
            #[inline]
            pub fn handles(
                &self,
            ) -> ::planus::Result<::core::option::Option<::planus::Vector<'a, u64>>> {
                self.0.access(2, "Call", "handles")
            }

            /// Getter for the [`resources` field](Call#structfield.resources).
            #[inline]
            pub fn resources(
                &self,
            ) -> ::planus::Result<::core::option::Option<::planus::Vector<'a, u64>>> {
                self.0.access(3, "Call", "resources")
            }

            /// Getter for the [`releases` field](Call#structfield.releases).
            #[inline]
            pub fn releases(
                &self,
            ) -> ::planus::Result<::core::option::Option<::planus::Vector<'a, u64>>> {
                self.0.access(4, "Call", "releases")
            }

            /// Getter for the [`arrays` field](Call#structfield.arrays).
            #[inline]
            pub fn arrays(
                &self,
            ) -> ::planus::Result<
                ::core::option::Option<
                    ::planus::Vector<'a, ::planus::Result<self::ArrowArrayRef<'a>>>,
                >,
            > {
                self.0.access(5, "Call", "arrays")
            }

            /// Getter for the [`data` field](Call#structfield.data).
            #[inline]
            pub fn data(
                &self,
            ) -> ::planus::Result<::core::option::Option<self::DataValueBoxRef<'a>>> {
                self.0.access(6, "Call", "data")
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
                if let ::core::option::Option::Some(field_handles) = self.handles().transpose() {
                    f.field("handles", &field_handles);
                }
                if let ::core::option::Option::Some(field_resources) = self.resources().transpose()
                {
                    f.field("resources", &field_resources);
                }
                if let ::core::option::Option::Some(field_releases) = self.releases().transpose() {
                    f.field("releases", &field_releases);
                }
                if let ::core::option::Option::Some(field_arrays) = self.arrays().transpose() {
                    f.field("arrays", &field_arrays);
                }
                if let ::core::option::Option::Some(field_data) = self.data().transpose() {
                    f.field("data", &field_data);
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
                    handles: if let ::core::option::Option::Some(handles) = value.handles()? {
                        ::core::option::Option::Some(handles.to_vec()?)
                    } else {
                        ::core::option::Option::None
                    },
                    resources: if let ::core::option::Option::Some(resources) = value.resources()? {
                        ::core::option::Option::Some(resources.to_vec()?)
                    } else {
                        ::core::option::Option::None
                    },
                    releases: if let ::core::option::Option::Some(releases) = value.releases()? {
                        ::core::option::Option::Some(releases.to_vec()?)
                    } else {
                        ::core::option::Option::None
                    },
                    arrays: if let ::core::option::Option::Some(arrays) = value.arrays()? {
                        ::core::option::Option::Some(arrays.to_vec_result()?)
                    } else {
                        ::core::option::Option::None
                    },
                    data: if let ::core::option::Option::Some(data) = value.data()? {
                        ::core::option::Option::Some(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryInto::try_into(data)?,
                        ))
                    } else {
                        ::core::option::Option::None
                    },
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

        /// The table `HandleList` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `HandleList` in the file `crates/quoin-ext-proto/schema/ext.fbs:77`
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
        pub struct HandleList {
            /// The field `handles` in the table `HandleList`
            pub handles: ::core::option::Option<::planus::alloc::vec::Vec<u64>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for HandleList {
            fn default() -> Self {
                Self {
                    handles: ::core::default::Default::default(),
                }
            }
        }

        impl HandleList {
            /// Creates a [HandleListBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> HandleListBuilder<()> {
                HandleListBuilder(())
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

        impl ::planus::WriteAs<::planus::Offset<HandleList>> for HandleList {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HandleList> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<HandleList>> for HandleList {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<HandleList>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<HandleList> for HandleList {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HandleList> {
                HandleList::create(builder, &self.handles)
            }
        }

        /// Builder for serializing an instance of the [HandleList] type.
        ///
        /// Can be created using the [HandleList::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct HandleListBuilder<State>(State);

        impl HandleListBuilder<()> {
            /// Setter for the [`handles` field](HandleList#structfield.handles).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handles<T0>(self, value: T0) -> HandleListBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            {
                HandleListBuilder((value,))
            }

            /// Sets the [`handles` field](HandleList#structfield.handles) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn handles_as_null(self) -> HandleListBuilder<((),)> {
                self.handles(())
            }
        }

        impl<T0> HandleListBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [HandleList].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<HandleList>
            where
                Self: ::planus::WriteAsOffset<HandleList>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>>
            ::planus::WriteAs<::planus::Offset<HandleList>> for HandleListBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<HandleList>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HandleList> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>>
            ::planus::WriteAsOptional<::planus::Offset<HandleList>> for HandleListBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<HandleList>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<HandleList>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>>
            ::planus::WriteAsOffset<HandleList> for HandleListBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<HandleList> {
                let (v0,) = &self.0;
                HandleList::create(builder, v0)
            }
        }

        /// Reference to a deserialized [HandleList].
        #[derive(Copy, Clone)]
        pub struct HandleListRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> HandleListRef<'a> {
            /// Getter for the [`handles` field](HandleList#structfield.handles).
            #[inline]
            pub fn handles(
                &self,
            ) -> ::planus::Result<::core::option::Option<::planus::Vector<'a, u64>>> {
                self.0.access(0, "HandleList", "handles")
            }
        }

        impl<'a> ::core::fmt::Debug for HandleListRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("HandleListRef");
                if let ::core::option::Option::Some(field_handles) = self.handles().transpose() {
                    f.field("handles", &field_handles);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<HandleListRef<'a>> for HandleList {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: HandleListRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    handles: if let ::core::option::Option::Some(handles) = value.handles()? {
                        ::core::option::Option::Some(handles.to_vec()?)
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for HandleListRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for HandleListRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[HandleListRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<HandleList>> for HandleList {
            type Value = ::planus::Offset<HandleList>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<HandleList>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for HandleListRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[HandleListRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `CallReturn` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `CallReturn` in the file `crates/quoin-ext-proto/schema/ext.fbs:82`
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

        /// The table `CallReturnResource` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `CallReturnResource` in the file `crates/quoin-ext-proto/schema/ext.fbs:88`
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
        pub struct CallReturnResource {
            /// The field `resource` in the table `CallReturnResource`
            pub resource: u64,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for CallReturnResource {
            fn default() -> Self {
                Self { resource: 0 }
            }
        }

        impl CallReturnResource {
            /// Creates a [CallReturnResourceBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> CallReturnResourceBuilder<()> {
                CallReturnResourceBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_resource: impl ::planus::WriteAsDefault<u64, u64>,
            ) -> ::planus::Offset<Self> {
                let prepared_resource = field_resource.prepare(builder, &0);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_resource.is_some() {
                    table_writer.write_entry::<u64>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_resource) = prepared_resource {
                            object_writer.write::<_, _, 8>(&prepared_resource);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<CallReturnResource>> for CallReturnResource {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallReturnResource> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<CallReturnResource>> for CallReturnResource {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<CallReturnResource>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<CallReturnResource> for CallReturnResource {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallReturnResource> {
                CallReturnResource::create(builder, self.resource)
            }
        }

        /// Builder for serializing an instance of the [CallReturnResource] type.
        ///
        /// Can be created using the [CallReturnResource::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct CallReturnResourceBuilder<State>(State);

        impl CallReturnResourceBuilder<()> {
            /// Setter for the [`resource` field](CallReturnResource#structfield.resource).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn resource<T0>(self, value: T0) -> CallReturnResourceBuilder<(T0,)>
            where
                T0: ::planus::WriteAsDefault<u64, u64>,
            {
                CallReturnResourceBuilder((value,))
            }

            /// Sets the [`resource` field](CallReturnResource#structfield.resource) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn resource_as_default(
                self,
            ) -> CallReturnResourceBuilder<(::planus::DefaultValue,)> {
                self.resource(::planus::DefaultValue)
            }
        }

        impl<T0> CallReturnResourceBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [CallReturnResource].
            #[inline]
            pub fn finish(
                self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallReturnResource>
            where
                Self: ::planus::WriteAsOffset<CallReturnResource>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<u64, u64>>
            ::planus::WriteAs<::planus::Offset<CallReturnResource>>
            for CallReturnResourceBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<CallReturnResource>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallReturnResource> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsDefault<u64, u64>>
            ::planus::WriteAsOptional<::planus::Offset<CallReturnResource>>
            for CallReturnResourceBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<CallReturnResource>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<CallReturnResource>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsDefault<u64, u64>> ::planus::WriteAsOffset<CallReturnResource>
            for CallReturnResourceBuilder<(T0,)>
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallReturnResource> {
                let (v0,) = &self.0;
                CallReturnResource::create(builder, v0)
            }
        }

        /// Reference to a deserialized [CallReturnResource].
        #[derive(Copy, Clone)]
        pub struct CallReturnResourceRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> CallReturnResourceRef<'a> {
            /// Getter for the [`resource` field](CallReturnResource#structfield.resource).
            #[inline]
            pub fn resource(&self) -> ::planus::Result<u64> {
                ::core::result::Result::Ok(
                    self.0
                        .access(0, "CallReturnResource", "resource")?
                        .unwrap_or(0),
                )
            }
        }

        impl<'a> ::core::fmt::Debug for CallReturnResourceRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("CallReturnResourceRef");
                f.field("resource", &self.resource());
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<CallReturnResourceRef<'a>> for CallReturnResource {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: CallReturnResourceRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    resource: ::core::convert::TryInto::try_into(value.resource()?)?,
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for CallReturnResourceRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for CallReturnResourceRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[CallReturnResourceRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<CallReturnResource>> for CallReturnResource {
            type Value = ::planus::Offset<CallReturnResource>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<CallReturnResource>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for CallReturnResourceRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[CallReturnResourceRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `CallReturnArray` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `CallReturnArray` in the file `crates/quoin-ext-proto/schema/ext.fbs:93`
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
        pub struct CallReturnArray {
            /// The field `array` in the table `CallReturnArray`
            pub array: ::core::option::Option<::planus::alloc::boxed::Box<self::ArrowArray>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for CallReturnArray {
            fn default() -> Self {
                Self {
                    array: ::core::default::Default::default(),
                }
            }
        }

        impl CallReturnArray {
            /// Creates a [CallReturnArrayBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> CallReturnArrayBuilder<()> {
                CallReturnArrayBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_array: impl ::planus::WriteAsOptional<::planus::Offset<self::ArrowArray>>,
            ) -> ::planus::Offset<Self> {
                let prepared_array = field_array.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_array.is_some() {
                    table_writer.write_entry::<::planus::Offset<self::ArrowArray>>(0);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_array) = prepared_array {
                            object_writer.write::<_, _, 4>(&prepared_array);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<CallReturnArray>> for CallReturnArray {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallReturnArray> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<CallReturnArray>> for CallReturnArray {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<CallReturnArray>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<CallReturnArray> for CallReturnArray {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallReturnArray> {
                CallReturnArray::create(builder, &self.array)
            }
        }

        /// Builder for serializing an instance of the [CallReturnArray] type.
        ///
        /// Can be created using the [CallReturnArray::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct CallReturnArrayBuilder<State>(State);

        impl CallReturnArrayBuilder<()> {
            /// Setter for the [`array` field](CallReturnArray#structfield.array).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn array<T0>(self, value: T0) -> CallReturnArrayBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<self::ArrowArray>>,
            {
                CallReturnArrayBuilder((value,))
            }

            /// Sets the [`array` field](CallReturnArray#structfield.array) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn array_as_null(self) -> CallReturnArrayBuilder<((),)> {
                self.array(())
            }
        }

        impl<T0> CallReturnArrayBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [CallReturnArray].
            #[inline]
            pub fn finish(
                self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallReturnArray>
            where
                Self: ::planus::WriteAsOffset<CallReturnArray>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<self::ArrowArray>>>
            ::planus::WriteAs<::planus::Offset<CallReturnArray>> for CallReturnArrayBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<CallReturnArray>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallReturnArray> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<self::ArrowArray>>>
            ::planus::WriteAsOptional<::planus::Offset<CallReturnArray>>
            for CallReturnArrayBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<CallReturnArray>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<CallReturnArray>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<self::ArrowArray>>>
            ::planus::WriteAsOffset<CallReturnArray> for CallReturnArrayBuilder<(T0,)>
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<CallReturnArray> {
                let (v0,) = &self.0;
                CallReturnArray::create(builder, v0)
            }
        }

        /// Reference to a deserialized [CallReturnArray].
        #[derive(Copy, Clone)]
        pub struct CallReturnArrayRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> CallReturnArrayRef<'a> {
            /// Getter for the [`array` field](CallReturnArray#structfield.array).
            #[inline]
            pub fn array(
                &self,
            ) -> ::planus::Result<::core::option::Option<self::ArrowArrayRef<'a>>> {
                self.0.access(0, "CallReturnArray", "array")
            }
        }

        impl<'a> ::core::fmt::Debug for CallReturnArrayRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("CallReturnArrayRef");
                if let ::core::option::Option::Some(field_array) = self.array().transpose() {
                    f.field("array", &field_array);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<CallReturnArrayRef<'a>> for CallReturnArray {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: CallReturnArrayRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    array: if let ::core::option::Option::Some(array) = value.array()? {
                        ::core::option::Option::Some(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryInto::try_into(array)?,
                        ))
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for CallReturnArrayRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for CallReturnArrayRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[CallReturnArrayRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<CallReturnArray>> for CallReturnArray {
            type Value = ::planus::Offset<CallReturnArray>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<CallReturnArray>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for CallReturnArrayRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[CallReturnArrayRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `CallReturnData` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `CallReturnData` in the file `crates/quoin-ext-proto/schema/ext.fbs:98`
        #[derive(Clone, Debug, PartialEq, PartialOrd, ::serde::Serialize, ::serde::Deserialize)]
        pub struct CallReturnData {
            /// The field `value` in the table `CallReturnData`
            pub value: ::core::option::Option<::planus::alloc::boxed::Box<self::DataValueBox>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for CallReturnData {
            fn default() -> Self {
                Self {
                    value: ::core::default::Default::default(),
                }
            }
        }

        impl CallReturnData {
            /// Creates a [CallReturnDataBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> CallReturnDataBuilder<()> {
                CallReturnDataBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_value: impl ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
            ) -> ::planus::Offset<Self> {
                let prepared_value = field_value.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<6> =
                    ::core::default::Default::default();
                if prepared_value.is_some() {
                    table_writer.write_entry::<::planus::Offset<self::DataValueBox>>(0);
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

        impl ::planus::WriteAs<::planus::Offset<CallReturnData>> for CallReturnData {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<CallReturnData> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<CallReturnData>> for CallReturnData {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<CallReturnData>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<CallReturnData> for CallReturnData {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<CallReturnData> {
                CallReturnData::create(builder, &self.value)
            }
        }

        /// Builder for serializing an instance of the [CallReturnData] type.
        ///
        /// Can be created using the [CallReturnData::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct CallReturnDataBuilder<State>(State);

        impl CallReturnDataBuilder<()> {
            /// Setter for the [`value` field](CallReturnData#structfield.value).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn value<T0>(self, value: T0) -> CallReturnDataBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>,
            {
                CallReturnDataBuilder((value,))
            }

            /// Sets the [`value` field](CallReturnData#structfield.value) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn value_as_null(self) -> CallReturnDataBuilder<((),)> {
                self.value(())
            }
        }

        impl<T0> CallReturnDataBuilder<(T0,)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [CallReturnData].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<CallReturnData>
            where
                Self: ::planus::WriteAsOffset<CallReturnData>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>>
            ::planus::WriteAs<::planus::Offset<CallReturnData>> for CallReturnDataBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<CallReturnData>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<CallReturnData> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>>
            ::planus::WriteAsOptional<::planus::Offset<CallReturnData>>
            for CallReturnDataBuilder<(T0,)>
        {
            type Prepared = ::planus::Offset<CallReturnData>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<CallReturnData>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<T0: ::planus::WriteAsOptional<::planus::Offset<self::DataValueBox>>>
            ::planus::WriteAsOffset<CallReturnData> for CallReturnDataBuilder<(T0,)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<CallReturnData> {
                let (v0,) = &self.0;
                CallReturnData::create(builder, v0)
            }
        }

        /// Reference to a deserialized [CallReturnData].
        #[derive(Copy, Clone)]
        pub struct CallReturnDataRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> CallReturnDataRef<'a> {
            /// Getter for the [`value` field](CallReturnData#structfield.value).
            #[inline]
            pub fn value(
                &self,
            ) -> ::planus::Result<::core::option::Option<self::DataValueBoxRef<'a>>> {
                self.0.access(0, "CallReturnData", "value")
            }
        }

        impl<'a> ::core::fmt::Debug for CallReturnDataRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("CallReturnDataRef");
                if let ::core::option::Option::Some(field_value) = self.value().transpose() {
                    f.field("value", &field_value);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<CallReturnDataRef<'a>> for CallReturnData {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: CallReturnDataRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    value: if let ::core::option::Option::Some(value) = value.value()? {
                        ::core::option::Option::Some(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryInto::try_into(value)?,
                        ))
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for CallReturnDataRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for CallReturnDataRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[CallReturnDataRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<CallReturnData>> for CallReturnData {
            type Value = ::planus::Offset<CallReturnData>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<CallReturnData>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for CallReturnDataRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[CallReturnDataRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `MakeString` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `MakeString` in the file `crates/quoin-ext-proto/schema/ext.fbs:103`
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
        /// * Table `HandleToString` in the file `crates/quoin-ext-proto/schema/ext.fbs:108`
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
        /// * Table `Retain` in the file `crates/quoin-ext-proto/schema/ext.fbs:114`
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
        /// * Table `Release` in the file `crates/quoin-ext-proto/schema/ext.fbs:119`
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
        /// * Table `CallMethodOnHandle` in the file `crates/quoin-ext-proto/schema/ext.fbs:127`
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

        /// The table `InvokeBlock` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `InvokeBlock` in the file `crates/quoin-ext-proto/schema/ext.fbs:136`
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
        pub struct InvokeBlock {
            /// The field `block` in the table `InvokeBlock`
            pub block: u64,
            /// The field `batches` in the table `InvokeBlock`
            pub batches: ::core::option::Option<::planus::alloc::vec::Vec<self::HandleList>>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for InvokeBlock {
            fn default() -> Self {
                Self {
                    block: 0,
                    batches: ::core::default::Default::default(),
                }
            }
        }

        impl InvokeBlock {
            /// Creates a [InvokeBlockBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> InvokeBlockBuilder<()> {
                InvokeBlockBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_block: impl ::planus::WriteAsDefault<u64, u64>,
                field_batches: impl ::planus::WriteAsOptional<
                    ::planus::Offset<[::planus::Offset<self::HandleList>]>,
                >,
            ) -> ::planus::Offset<Self> {
                let prepared_block = field_block.prepare(builder, &0);
                let prepared_batches = field_batches.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<8> =
                    ::core::default::Default::default();
                if prepared_block.is_some() {
                    table_writer.write_entry::<u64>(0);
                }
                if prepared_batches.is_some() {
                    table_writer
                        .write_entry::<::planus::Offset<[::planus::Offset<self::HandleList>]>>(1);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_block) = prepared_block {
                            object_writer.write::<_, _, 8>(&prepared_block);
                        }
                        if let ::core::option::Option::Some(prepared_batches) = prepared_batches {
                            object_writer.write::<_, _, 4>(&prepared_batches);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<InvokeBlock>> for InvokeBlock {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<InvokeBlock> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<InvokeBlock>> for InvokeBlock {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<InvokeBlock>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<InvokeBlock> for InvokeBlock {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<InvokeBlock> {
                InvokeBlock::create(builder, self.block, &self.batches)
            }
        }

        /// Builder for serializing an instance of the [InvokeBlock] type.
        ///
        /// Can be created using the [InvokeBlock::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct InvokeBlockBuilder<State>(State);

        impl InvokeBlockBuilder<()> {
            /// Setter for the [`block` field](InvokeBlock#structfield.block).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn block<T0>(self, value: T0) -> InvokeBlockBuilder<(T0,)>
            where
                T0: ::planus::WriteAsDefault<u64, u64>,
            {
                InvokeBlockBuilder((value,))
            }

            /// Sets the [`block` field](InvokeBlock#structfield.block) to the default value.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn block_as_default(self) -> InvokeBlockBuilder<(::planus::DefaultValue,)> {
                self.block(::planus::DefaultValue)
            }
        }

        impl<T0> InvokeBlockBuilder<(T0,)> {
            /// Setter for the [`batches` field](InvokeBlock#structfield.batches).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn batches<T1>(self, value: T1) -> InvokeBlockBuilder<(T0, T1)>
            where
                T1: ::planus::WriteAsOptional<
                        ::planus::Offset<[::planus::Offset<self::HandleList>]>,
                    >,
            {
                let (v0,) = self.0;
                InvokeBlockBuilder((v0, value))
            }

            /// Sets the [`batches` field](InvokeBlock#structfield.batches) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn batches_as_null(self) -> InvokeBlockBuilder<(T0, ())> {
                self.batches(())
            }
        }

        impl<T0, T1> InvokeBlockBuilder<(T0, T1)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [InvokeBlock].
            #[inline]
            pub fn finish(self, builder: &mut ::planus::Builder) -> ::planus::Offset<InvokeBlock>
            where
                Self: ::planus::WriteAsOffset<InvokeBlock>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<u64, u64>,
            T1: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::HandleList>]>>,
        > ::planus::WriteAs<::planus::Offset<InvokeBlock>> for InvokeBlockBuilder<(T0, T1)>
        {
            type Prepared = ::planus::Offset<InvokeBlock>;

            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<InvokeBlock> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<u64, u64>,
            T1: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::HandleList>]>>,
        > ::planus::WriteAsOptional<::planus::Offset<InvokeBlock>>
            for InvokeBlockBuilder<(T0, T1)>
        {
            type Prepared = ::planus::Offset<InvokeBlock>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<InvokeBlock>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<
            T0: ::planus::WriteAsDefault<u64, u64>,
            T1: ::planus::WriteAsOptional<::planus::Offset<[::planus::Offset<self::HandleList>]>>,
        > ::planus::WriteAsOffset<InvokeBlock> for InvokeBlockBuilder<(T0, T1)>
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::Offset<InvokeBlock> {
                let (v0, v1) = &self.0;
                InvokeBlock::create(builder, v0, v1)
            }
        }

        /// Reference to a deserialized [InvokeBlock].
        #[derive(Copy, Clone)]
        pub struct InvokeBlockRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> InvokeBlockRef<'a> {
            /// Getter for the [`block` field](InvokeBlock#structfield.block).
            #[inline]
            pub fn block(&self) -> ::planus::Result<u64> {
                ::core::result::Result::Ok(self.0.access(0, "InvokeBlock", "block")?.unwrap_or(0))
            }

            /// Getter for the [`batches` field](InvokeBlock#structfield.batches).
            #[inline]
            pub fn batches(
                &self,
            ) -> ::planus::Result<
                ::core::option::Option<
                    ::planus::Vector<'a, ::planus::Result<self::HandleListRef<'a>>>,
                >,
            > {
                self.0.access(1, "InvokeBlock", "batches")
            }
        }

        impl<'a> ::core::fmt::Debug for InvokeBlockRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("InvokeBlockRef");
                f.field("block", &self.block());
                if let ::core::option::Option::Some(field_batches) = self.batches().transpose() {
                    f.field("batches", &field_batches);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<InvokeBlockRef<'a>> for InvokeBlock {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: InvokeBlockRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    block: ::core::convert::TryInto::try_into(value.block()?)?,
                    batches: if let ::core::option::Option::Some(batches) = value.batches()? {
                        ::core::option::Option::Some(batches.to_vec_result()?)
                    } else {
                        ::core::option::Option::None
                    },
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for InvokeBlockRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for InvokeBlockRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[InvokeBlockRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<InvokeBlock>> for InvokeBlock {
            type Value = ::planus::Offset<InvokeBlock>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<InvokeBlock>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for InvokeBlockRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[InvokeBlockRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `InvokeBlockReturn` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `InvokeBlockReturn` in the file `crates/quoin-ext-proto/schema/ext.fbs:143`
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
        pub struct InvokeBlockReturn {
            /// The field `results` in the table `InvokeBlockReturn`
            pub results: ::core::option::Option<::planus::alloc::vec::Vec<u64>>,
            /// The field `error` in the table `InvokeBlockReturn`
            pub error: ::core::option::Option<::planus::alloc::string::String>,
        }

        #[allow(clippy::derivable_impls)]
        impl ::core::default::Default for InvokeBlockReturn {
            fn default() -> Self {
                Self {
                    results: ::core::default::Default::default(),
                    error: ::core::default::Default::default(),
                }
            }
        }

        impl InvokeBlockReturn {
            /// Creates a [InvokeBlockReturnBuilder] for serializing an instance of this table.
            #[inline]
            pub fn builder() -> InvokeBlockReturnBuilder<()> {
                InvokeBlockReturnBuilder(())
            }

            #[allow(clippy::too_many_arguments)]
            pub fn create(
                builder: &mut ::planus::Builder,
                field_results: impl ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
                field_error: impl ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            ) -> ::planus::Offset<Self> {
                let prepared_results = field_results.prepare(builder);
                let prepared_error = field_error.prepare(builder);

                let mut table_writer: ::planus::table_writer::TableWriter<8> =
                    ::core::default::Default::default();
                if prepared_results.is_some() {
                    table_writer.write_entry::<::planus::Offset<[u64]>>(0);
                }
                if prepared_error.is_some() {
                    table_writer.write_entry::<::planus::Offset<str>>(1);
                }

                unsafe {
                    table_writer.finish(builder, |object_writer| {
                        if let ::core::option::Option::Some(prepared_results) = prepared_results {
                            object_writer.write::<_, _, 4>(&prepared_results);
                        }
                        if let ::core::option::Option::Some(prepared_error) = prepared_error {
                            object_writer.write::<_, _, 4>(&prepared_error);
                        }
                    });
                }
                builder.current_offset()
            }
        }

        impl ::planus::WriteAs<::planus::Offset<InvokeBlockReturn>> for InvokeBlockReturn {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<InvokeBlockReturn> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl ::planus::WriteAsOptional<::planus::Offset<InvokeBlockReturn>> for InvokeBlockReturn {
            type Prepared = ::planus::Offset<Self>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<InvokeBlockReturn>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl ::planus::WriteAsOffset<InvokeBlockReturn> for InvokeBlockReturn {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<InvokeBlockReturn> {
                InvokeBlockReturn::create(builder, &self.results, &self.error)
            }
        }

        /// Builder for serializing an instance of the [InvokeBlockReturn] type.
        ///
        /// Can be created using the [InvokeBlockReturn::builder] method.
        #[derive(Debug)]
        #[must_use]
        pub struct InvokeBlockReturnBuilder<State>(State);

        impl InvokeBlockReturnBuilder<()> {
            /// Setter for the [`results` field](InvokeBlockReturn#structfield.results).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn results<T0>(self, value: T0) -> InvokeBlockReturnBuilder<(T0,)>
            where
                T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            {
                InvokeBlockReturnBuilder((value,))
            }

            /// Sets the [`results` field](InvokeBlockReturn#structfield.results) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn results_as_null(self) -> InvokeBlockReturnBuilder<((),)> {
                self.results(())
            }
        }

        impl<T0> InvokeBlockReturnBuilder<(T0,)> {
            /// Setter for the [`error` field](InvokeBlockReturn#structfield.error).
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn error<T1>(self, value: T1) -> InvokeBlockReturnBuilder<(T0, T1)>
            where
                T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
            {
                let (v0,) = self.0;
                InvokeBlockReturnBuilder((v0, value))
            }

            /// Sets the [`error` field](InvokeBlockReturn#structfield.error) to null.
            #[inline]
            #[allow(clippy::type_complexity)]
            pub fn error_as_null(self) -> InvokeBlockReturnBuilder<(T0, ())> {
                self.error(())
            }
        }

        impl<T0, T1> InvokeBlockReturnBuilder<(T0, T1)> {
            /// Finish writing the builder to get an [Offset](::planus::Offset) to a serialized [InvokeBlockReturn].
            #[inline]
            pub fn finish(
                self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<InvokeBlockReturn>
            where
                Self: ::planus::WriteAsOffset<InvokeBlockReturn>,
            {
                ::planus::WriteAsOffset::prepare(&self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAs<::planus::Offset<InvokeBlockReturn>>
            for InvokeBlockReturnBuilder<(T0, T1)>
        {
            type Prepared = ::planus::Offset<InvokeBlockReturn>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<InvokeBlockReturn> {
                ::planus::WriteAsOffset::prepare(self, builder)
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAsOptional<::planus::Offset<InvokeBlockReturn>>
            for InvokeBlockReturnBuilder<(T0, T1)>
        {
            type Prepared = ::planus::Offset<InvokeBlockReturn>;

            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::Offset<InvokeBlockReturn>> {
                ::core::option::Option::Some(::planus::WriteAsOffset::prepare(self, builder))
            }
        }

        impl<
            T0: ::planus::WriteAsOptional<::planus::Offset<[u64]>>,
            T1: ::planus::WriteAsOptional<::planus::Offset<::core::primitive::str>>,
        > ::planus::WriteAsOffset<InvokeBlockReturn> for InvokeBlockReturnBuilder<(T0, T1)>
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::planus::Offset<InvokeBlockReturn> {
                let (v0, v1) = &self.0;
                InvokeBlockReturn::create(builder, v0, v1)
            }
        }

        /// Reference to a deserialized [InvokeBlockReturn].
        #[derive(Copy, Clone)]
        pub struct InvokeBlockReturnRef<'a>(#[allow(dead_code)] ::planus::table_reader::Table<'a>);

        impl<'a> InvokeBlockReturnRef<'a> {
            /// Getter for the [`results` field](InvokeBlockReturn#structfield.results).
            #[inline]
            pub fn results(
                &self,
            ) -> ::planus::Result<::core::option::Option<::planus::Vector<'a, u64>>> {
                self.0.access(0, "InvokeBlockReturn", "results")
            }

            /// Getter for the [`error` field](InvokeBlockReturn#structfield.error).
            #[inline]
            pub fn error(
                &self,
            ) -> ::planus::Result<::core::option::Option<&'a ::core::primitive::str>> {
                self.0.access(1, "InvokeBlockReturn", "error")
            }
        }

        impl<'a> ::core::fmt::Debug for InvokeBlockReturnRef<'a> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut f = f.debug_struct("InvokeBlockReturnRef");
                if let ::core::option::Option::Some(field_results) = self.results().transpose() {
                    f.field("results", &field_results);
                }
                if let ::core::option::Option::Some(field_error) = self.error().transpose() {
                    f.field("error", &field_error);
                }
                f.finish()
            }
        }

        impl<'a> ::core::convert::TryFrom<InvokeBlockReturnRef<'a>> for InvokeBlockReturn {
            type Error = ::planus::Error;

            #[allow(unreachable_code)]
            fn try_from(value: InvokeBlockReturnRef<'a>) -> ::planus::Result<Self> {
                ::core::result::Result::Ok(Self {
                    results: if let ::core::option::Option::Some(results) = value.results()? {
                        ::core::option::Option::Some(results.to_vec()?)
                    } else {
                        ::core::option::Option::None
                    },
                    error: value.error()?.map(::core::convert::Into::into),
                })
            }
        }

        impl<'a> ::planus::TableRead<'a> for InvokeBlockReturnRef<'a> {
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

        impl<'a> ::planus::VectorReadInner<'a> for InvokeBlockReturnRef<'a> {
            type Error = ::planus::Error;
            const STRIDE: usize = 4;

            unsafe fn from_buffer(
                buffer: ::planus::SliceWithStartOffset<'a>,
                offset: usize,
            ) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(buffer, offset).map_err(|error_kind| {
                    error_kind.with_error_location(
                        "[InvokeBlockReturnRef]",
                        "get",
                        buffer.offset_from_start,
                    )
                })
            }
        }

        /// # Safety
        /// The planus compiler generates implementations that initialize
        /// the bytes in `write_values`.
        unsafe impl ::planus::VectorWrite<::planus::Offset<InvokeBlockReturn>> for InvokeBlockReturn {
            type Value = ::planus::Offset<InvokeBlockReturn>;
            const STRIDE: usize = 4;
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> Self::Value {
                ::planus::WriteAs::prepare(self, builder)
            }

            #[inline]
            unsafe fn write_values(
                values: &[::planus::Offset<InvokeBlockReturn>],
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

        impl<'a> ::planus::ReadAsRoot<'a> for InvokeBlockReturnRef<'a> {
            fn read_as_root(slice: &'a [u8]) -> ::planus::Result<Self> {
                ::planus::TableRead::from_buffer(
                    ::planus::SliceWithStartOffset {
                        buffer: slice,
                        offset_from_start: 0,
                    },
                    0,
                )
                .map_err(|error_kind| {
                    error_kind.with_error_location("[InvokeBlockReturnRef]", "read_as_root", 0)
                })
            }
        }

        /// The table `HostOpReturn` in the namespace `quoin_ext_proto`
        ///
        /// Generated from these locations:
        /// * Table `HostOpReturn` in the file `crates/quoin-ext-proto/schema/ext.fbs:151`
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
        /// * Union `Message` in the file `crates/quoin-ext-proto/schema/ext.fbs:157`
        #[derive(Clone, Debug, PartialEq, PartialOrd, ::serde::Serialize, ::serde::Deserialize)]
        pub enum Message {
            /// The variant of type `Call` in the union `Message`
            Call(::planus::alloc::boxed::Box<self::Call>),

            /// The variant of type `CallReturn` in the union `Message`
            CallReturn(::planus::alloc::boxed::Box<self::CallReturn>),

            /// The variant of type `CallReturnResource` in the union `Message`
            CallReturnResource(::planus::alloc::boxed::Box<self::CallReturnResource>),

            /// The variant of type `CallReturnArray` in the union `Message`
            CallReturnArray(::planus::alloc::boxed::Box<self::CallReturnArray>),

            /// The variant of type `CallReturnData` in the union `Message`
            CallReturnData(::planus::alloc::boxed::Box<self::CallReturnData>),

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

            /// The variant of type `InvokeBlock` in the union `Message`
            InvokeBlock(::planus::alloc::boxed::Box<self::InvokeBlock>),

            /// The variant of type `InvokeBlockReturn` in the union `Message`
            InvokeBlockReturn(::planus::alloc::boxed::Box<self::InvokeBlockReturn>),

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
            pub fn create_call_return_resource(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::CallReturnResource>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(3, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_call_return_array(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::CallReturnArray>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(4, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_call_return_data(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::CallReturnData>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(5, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_make_string(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::MakeString>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(6, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_handle_to_string(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::HandleToString>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(7, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_retain(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::Retain>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(8, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_release(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::Release>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(9, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_call_method_on_handle(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::CallMethodOnHandle>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(10, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_invoke_block(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::InvokeBlock>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(11, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_invoke_block_return(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::InvokeBlockReturn>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(12, value.prepare(builder).downcast())
            }

            #[inline]
            pub fn create_host_op_return(
                builder: &mut ::planus::Builder,
                value: impl ::planus::WriteAsOffset<self::HostOpReturn>,
            ) -> ::planus::UnionOffset<Self> {
                ::planus::UnionOffset::new(13, value.prepare(builder).downcast())
            }
        }

        impl ::planus::WriteAsUnion<Message> for Message {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Self> {
                match self {
                    Self::Call(value) => Self::create_call(builder, value),
                    Self::CallReturn(value) => Self::create_call_return(builder, value),
                    Self::CallReturnResource(value) => {
                        Self::create_call_return_resource(builder, value)
                    }
                    Self::CallReturnArray(value) => Self::create_call_return_array(builder, value),
                    Self::CallReturnData(value) => Self::create_call_return_data(builder, value),
                    Self::MakeString(value) => Self::create_make_string(builder, value),
                    Self::HandleToString(value) => Self::create_handle_to_string(builder, value),
                    Self::Retain(value) => Self::create_retain(builder, value),
                    Self::Release(value) => Self::create_release(builder, value),
                    Self::CallMethodOnHandle(value) => {
                        Self::create_call_method_on_handle(builder, value)
                    }
                    Self::InvokeBlock(value) => Self::create_invoke_block(builder, value),
                    Self::InvokeBlockReturn(value) => {
                        Self::create_invoke_block_return(builder, value)
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

            /// Creates an instance of the [`CallReturnResource` variant](Message#variant.CallReturnResource).
            #[inline]
            pub fn call_return_resource<T>(
                self,
                value: T,
            ) -> MessageBuilder<::planus::Initialized<3, T>>
            where
                T: ::planus::WriteAsOffset<self::CallReturnResource>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`CallReturnArray` variant](Message#variant.CallReturnArray).
            #[inline]
            pub fn call_return_array<T>(
                self,
                value: T,
            ) -> MessageBuilder<::planus::Initialized<4, T>>
            where
                T: ::planus::WriteAsOffset<self::CallReturnArray>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`CallReturnData` variant](Message#variant.CallReturnData).
            #[inline]
            pub fn call_return_data<T>(
                self,
                value: T,
            ) -> MessageBuilder<::planus::Initialized<5, T>>
            where
                T: ::planus::WriteAsOffset<self::CallReturnData>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`MakeString` variant](Message#variant.MakeString).
            #[inline]
            pub fn make_string<T>(self, value: T) -> MessageBuilder<::planus::Initialized<6, T>>
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
            ) -> MessageBuilder<::planus::Initialized<7, T>>
            where
                T: ::planus::WriteAsOffset<self::HandleToString>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`Retain` variant](Message#variant.Retain).
            #[inline]
            pub fn retain<T>(self, value: T) -> MessageBuilder<::planus::Initialized<8, T>>
            where
                T: ::planus::WriteAsOffset<self::Retain>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`Release` variant](Message#variant.Release).
            #[inline]
            pub fn release<T>(self, value: T) -> MessageBuilder<::planus::Initialized<9, T>>
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
            ) -> MessageBuilder<::planus::Initialized<10, T>>
            where
                T: ::planus::WriteAsOffset<self::CallMethodOnHandle>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`InvokeBlock` variant](Message#variant.InvokeBlock).
            #[inline]
            pub fn invoke_block<T>(self, value: T) -> MessageBuilder<::planus::Initialized<11, T>>
            where
                T: ::planus::WriteAsOffset<self::InvokeBlock>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`InvokeBlockReturn` variant](Message#variant.InvokeBlockReturn).
            #[inline]
            pub fn invoke_block_return<T>(
                self,
                value: T,
            ) -> MessageBuilder<::planus::Initialized<12, T>>
            where
                T: ::planus::WriteAsOffset<self::InvokeBlockReturn>,
            {
                MessageBuilder(::planus::Initialized(value))
            }

            /// Creates an instance of the [`HostOpReturn` variant](Message#variant.HostOpReturn).
            #[inline]
            pub fn host_op_return<T>(self, value: T) -> MessageBuilder<::planus::Initialized<13, T>>
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
            T: ::planus::WriteAsOffset<self::CallReturnResource>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(3, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<3, T>>
        where
            T: ::planus::WriteAsOffset<self::CallReturnResource>,
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
            T: ::planus::WriteAsOffset<self::CallReturnArray>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(4, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<4, T>>
        where
            T: ::planus::WriteAsOffset<self::CallReturnArray>,
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
            T: ::planus::WriteAsOffset<self::CallReturnData>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(5, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<5, T>>
        where
            T: ::planus::WriteAsOffset<self::CallReturnData>,
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
            T: ::planus::WriteAsOffset<self::MakeString>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(6, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<6, T>>
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
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<7, T>>
        where
            T: ::planus::WriteAsOffset<self::HandleToString>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(7, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<7, T>>
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
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<8, T>>
        where
            T: ::planus::WriteAsOffset<self::Retain>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(8, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<8, T>>
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
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<9, T>>
        where
            T: ::planus::WriteAsOffset<self::Release>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(9, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<9, T>>
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
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<10, T>>
        where
            T: ::planus::WriteAsOffset<self::CallMethodOnHandle>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(10, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<10, T>>
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
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<11, T>>
        where
            T: ::planus::WriteAsOffset<self::InvokeBlock>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(11, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<11, T>>
        where
            T: ::planus::WriteAsOffset<self::InvokeBlock>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Message>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<12, T>>
        where
            T: ::planus::WriteAsOffset<self::InvokeBlockReturn>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(12, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<12, T>>
        where
            T: ::planus::WriteAsOffset<self::InvokeBlockReturn>,
        {
            #[inline]
            fn prepare(
                &self,
                builder: &mut ::planus::Builder,
            ) -> ::core::option::Option<::planus::UnionOffset<Message>> {
                ::core::option::Option::Some(::planus::WriteAsUnion::prepare(self, builder))
            }
        }
        impl<T> ::planus::WriteAsUnion<Message> for MessageBuilder<::planus::Initialized<13, T>>
        where
            T: ::planus::WriteAsOffset<self::HostOpReturn>,
        {
            #[inline]
            fn prepare(&self, builder: &mut ::planus::Builder) -> ::planus::UnionOffset<Message> {
                ::planus::UnionOffset::new(13, (self.0).0.prepare(builder).downcast())
            }
        }

        impl<T> ::planus::WriteAsOptionalUnion<Message> for MessageBuilder<::planus::Initialized<13, T>>
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
            CallReturnResource(self::CallReturnResourceRef<'a>),
            CallReturnArray(self::CallReturnArrayRef<'a>),
            CallReturnData(self::CallReturnDataRef<'a>),
            MakeString(self::MakeStringRef<'a>),
            HandleToString(self::HandleToStringRef<'a>),
            Retain(self::RetainRef<'a>),
            Release(self::ReleaseRef<'a>),
            CallMethodOnHandle(self::CallMethodOnHandleRef<'a>),
            InvokeBlock(self::InvokeBlockRef<'a>),
            InvokeBlockReturn(self::InvokeBlockReturnRef<'a>),
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

                    MessageRef::CallReturnResource(value) => {
                        Self::CallReturnResource(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    MessageRef::CallReturnArray(value) => {
                        Self::CallReturnArray(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    MessageRef::CallReturnData(value) => {
                        Self::CallReturnData(::planus::alloc::boxed::Box::new(
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

                    MessageRef::InvokeBlock(value) => {
                        Self::InvokeBlock(::planus::alloc::boxed::Box::new(
                            ::core::convert::TryFrom::try_from(value)?,
                        ))
                    }

                    MessageRef::InvokeBlockReturn(value) => {
                        Self::InvokeBlockReturn(::planus::alloc::boxed::Box::new(
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
                    3 => ::core::result::Result::Ok(Self::CallReturnResource(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    4 => ::core::result::Result::Ok(Self::CallReturnArray(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    5 => ::core::result::Result::Ok(Self::CallReturnData(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    6 => ::core::result::Result::Ok(Self::MakeString(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    7 => ::core::result::Result::Ok(Self::HandleToString(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    8 => ::core::result::Result::Ok(Self::Retain(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    9 => ::core::result::Result::Ok(Self::Release(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    10 => ::core::result::Result::Ok(Self::CallMethodOnHandle(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    11 => ::core::result::Result::Ok(Self::InvokeBlock(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    12 => ::core::result::Result::Ok(Self::InvokeBlockReturn(
                        ::planus::TableRead::from_buffer(buffer, field_offset)?,
                    )),
                    13 => ::core::result::Result::Ok(Self::HostOpReturn(
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
        /// * Table `Envelope` in the file `crates/quoin-ext-proto/schema/ext.fbs:173`
        #[derive(Clone, Debug, PartialEq, PartialOrd, ::serde::Serialize, ::serde::Deserialize)]
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
