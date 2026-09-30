/// Something the toolbar or a keyboard shortcut asks the editor to do to the text: the selection,
/// or the word at the cursor if nothing is selected.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Bold,
    Italic,
    Strikethrough,
    Superscript,
    Subscript,
    Uppercase,
    /// Level 1 to 6; the same level again makes it a paragraph.
    Heading(usize),
    BulletList,
    NumberedList,
    Quote,
    Link,
}
