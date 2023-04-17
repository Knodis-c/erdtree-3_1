use crate::{
    fs::{
        inode::Inode,
        permissions::{FileMode, SymbolicNotation},
        xattr::ExtendedAttr,
    },
    icons,
    render::{
        context::Context,
        disk_usage::file_size::{DiskUsage, FileSize},
        styles::get_ls_colors,
        tree::error::Error,
    },
};
use ansi_term::Style;
use ignore::DirEntry;
use lscolors::Style as LS_Style;
use std::{
    borrow::Cow,
    convert::TryFrom,
    ffi::OsStr,
    fmt::{self, Formatter},
    fs::{FileType, Metadata},
    path::{Path, PathBuf},
};
use xattr::XAttrs;

/// Ordering and sorting rules for [Node].
pub mod cmp;

/// For building the actual output.
pub mod output;

/// All methods of [Node] that pertain to styling the output.
pub mod style;

/// A node of [`Tree`] that can be created from a [DirEntry]. Any filesystem I/O and
/// relevant system calls are expected to complete after initialization. A `Node` when `Display`ed
/// uses ANSI colors determined by the file-type and `LS_COLORS`.
///
/// [`Tree`]: super::Tree
pub struct Node {
    dir_entry: DirEntry,
    metadata: Metadata,
    file_size: Option<FileSize>,
    style: Option<Style>,
    symlink_target: Option<PathBuf>,
    inode: Option<Inode>,

    /// Will always be `None` on incompatible platforms.
    xattrs: Option<XAttrs>,
}

impl Node {
    /// Initializes a new [Node].
    pub const fn new(
        dir_entry: DirEntry,
        metadata: Metadata,
        file_size: Option<FileSize>,
        style: Option<Style>,
        symlink_target: Option<PathBuf>,
        inode: Option<Inode>,
        xattrs: Option<XAttrs>,
    ) -> Self {
        Self {
            dir_entry,
            metadata,
            file_size,
            style,
            symlink_target,
            inode,
            xattrs,
        }
    }

    /// Returns a reference to `file_name`. If file is a symlink then `file_name` is the name of
    /// the symlink not the target.
    pub fn file_name(&self) -> &OsStr {
        self.dir_entry.file_name()
    }

    pub const fn dir_entry(&self) -> &DirEntry {
        &self.dir_entry
    }

    /// Get depth level of [Node].
    pub fn depth(&self) -> usize {
        self.dir_entry.depth()
    }

    /// Gets the underlying [Inode] of the entry.
    pub const fn inode(&self) -> Option<Inode> {
        self.inode
    }

    /// Returns the underlying `ino` of the [DirEntry].
    pub fn ino(&self) -> Option<u64> {
        self.inode.map(|inode| inode.ino)
    }

    /// Returns the underlying `nlink` of the [DirEntry].
    pub fn nlink(&self) -> Option<u64> {
        self.inode.map(|inode| inode.nlink)
    }

    /// Converts `OsStr` to `String`; if fails does a lossy conversion replacing non-Unicode
    /// sequences with Unicode replacement scalar values.
    pub fn file_name_lossy(&self) -> Cow<'_, str> {
        self.file_name()
            .to_str()
            .map_or_else(|| self.file_name().to_string_lossy(), Cow::from)
    }

    /// Returns `true` if node is a directory.
    pub fn is_dir(&self) -> bool {
        self.file_type().map_or(false, |ft| ft.is_dir())
    }

    /// Is the Node a symlink.
    pub const fn is_symlink(&self) -> bool {
        self.symlink_target.is_some()
    }

    /// Path to symlink target.
    pub fn symlink_target_path(&self) -> Option<&Path> {
        self.symlink_target.as_deref()
    }

    /// Returns the file name of the symlink target if [Node] represents a symlink.
    pub fn symlink_target_file_name(&self) -> Option<&OsStr> {
        self.symlink_target_path().and_then(Path::file_name)
    }

    /// Returns reference to underlying [FileType].
    pub fn file_type(&self) -> Option<FileType> {
        self.dir_entry.file_type()
    }

    /// Returns the path to the [Node]'s parent, if any.
    pub fn parent_path(&self) -> Option<&Path> {
        self.path().parent()
    }

    /// Returns a reference to `path`.
    pub fn path(&self) -> &Path {
        self.dir_entry.path()
    }

    /// Gets 'file_size'.
    pub const fn file_size(&self) -> Option<&FileSize> {
        self.file_size.as_ref()
    }

    /// Sets `file_size`.
    pub fn set_file_size(&mut self, size: FileSize) {
        self.file_size = Some(size);
    }

    /// Attempts to return an instance of [FileMode] for the display of symbolic permissions.
    pub fn mode(&self) -> Result<FileMode, Error> {
        let permissions = self.metadata.permissions();
        let file_mode = permissions.try_mode_symbolic_notation()?;
        Ok(file_mode)
    }

    /// Whether or not [Node] has extended attributes.
    ///
    /// TODO: Cloning can potentially be expensive here, but practically speaking we won't run into
    /// this scenario a lot, but we will want to optimize this bad boy by removing the `xattr`
    /// crate and just query for the existence of xattrs ourselves.
    fn has_xattrs(&self) -> bool {
        let count = self
            .xattrs
            .as_ref()
            .map_or(0, |xattrs| xattrs.clone().count());

        count > 0
    }

    /// General method for printing a `Node`. The `Display` (and `ToString`) traits are not used,
    /// to give more control over the output.
    ///
    /// Format a node for display with size on the right.
    ///
    /// Example:
    /// `| Some Directory (12.3 KiB)`
    ///
    ///
    /// Format a node for display with size on the left.
    ///
    /// Example:
    /// `  1.23 MiB | Some File`
    ///
    /// Note the two spaces to the left of the first character of the number -- even if never used,
    /// numbers are padded to 3 digits to the left of the decimal (and ctx.scale digits after)
    pub fn display(&self, f: &mut Formatter, prefix: &str, ctx: &Context) -> fmt::Result {
        let out = if ctx.no_color() {
            output::compute(self, prefix, ctx)
        } else {
            output::compute_with_color(self, prefix, ctx)
        };

        write!(f, "{out}")
    }

    /// Unix file identifiers that you'd find in the `ls -l` command.
    #[cfg(unix)]
    pub fn file_type_identifier(&self) -> Option<&str> {
        use std::os::unix::fs::FileTypeExt;

        let file_type = self.file_type()?;

        let iden = if file_type.is_dir() {
            "d"
        } else if file_type.is_file() {
            "-"
        } else if file_type.is_symlink() {
            "l"
        } else if file_type.is_fifo() {
            "p"
        } else if file_type.is_socket() {
            "s"
        } else if file_type.is_char_device() {
            "c"
        } else if file_type.is_block_device() {
            "b"
        } else {
            return None;
        };

        Some(iden)
    }

    /// File identifiers.
    #[cfg(not(unix))]
    pub fn file_type_identifier(&self) -> Option<&str> {
        let file_type = self.file_type()?;

        let iden = if file_type.is_dir() {
            "d"
        } else if file_type.is_file() {
            "-"
        } else if file_type.is_symlink() {
            "l"
        } else {
            return None;
        };

        Some(iden)
    }

    /// See [icons::compute].
    fn compute_icon(&self, no_color: bool) -> Cow<'static, str> {
        if no_color {
            icons::compute(self.dir_entry(), self.symlink_target_path())
        } else {
            icons::compute_with_color(self.dir_entry(), self.symlink_target_path(), self.style)
        }
    }
}

impl TryFrom<(DirEntry, &Context)> for Node {
    type Error = Error;

    fn try_from(data: (DirEntry, &Context)) -> Result<Self, Error> {
        let (dir_entry, ctx) = data;

        let path = dir_entry.path();

        let link_target = crate::fs::symlink_target(&dir_entry);

        let metadata = dir_entry.metadata()?;

        let style = get_ls_colors().ok().and_then(|ls_colors| {
            ls_colors
                .style_for_path_with_metadata(path, Some(&metadata))
                .map(LS_Style::to_ansi_term_style)
                .or_else(|| Some(Style::default()))
        });

        let file_type = dir_entry.file_type();

        let file_size = match file_type {
            Some(ref ft) if ft.is_file() && !ctx.suppress_size => match ctx.disk_usage {
                DiskUsage::Logical => Some(FileSize::logical(&metadata, ctx.unit, ctx.scale)),
                DiskUsage::Physical => FileSize::physical(path, &metadata, ctx.unit, ctx.scale),
            },
            _ => None,
        };

        let inode = Inode::try_from(&metadata).ok();

        let xattrs = if ctx.long {
            dir_entry.get_xattrs()
        } else {
            None
        };

        Ok(Self::new(
            dir_entry,
            metadata,
            file_size,
            style,
            link_target,
            inode,
            xattrs,
        ))
    }
}
