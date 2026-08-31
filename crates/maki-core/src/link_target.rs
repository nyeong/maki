#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentSelector<'a> {
    Current,
    Legacy(&'a str),
    Root(&'a str),
    Child(&'a str),
}

impl<'a> DocumentSelector<'a> {
    pub fn target(self) -> Option<&'a str> {
        match self {
            Self::Current => None,
            Self::Legacy(target) | Self::Root(target) | Self::Child(target) => Some(target),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InnerSelector<'a> {
    Heading(&'a str),
    Id(&'a str),
}

impl<'a> InnerSelector<'a> {
    pub fn target(self) -> &'a str {
        match self {
            Self::Heading(target) | Self::Id(target) => target,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteLinkTarget<'a> {
    pub document: DocumentSelector<'a>,
    pub inner: Option<InnerSelector<'a>>,
}

impl<'a> NoteLinkTarget<'a> {
    pub fn parse(target: &'a str) -> Self {
        let delimiter = target
            .char_indices()
            .find(|(_, character)| matches!(character, '#' | '@'));
        let (document, inner) = delimiter.map_or((target, None), |(index, delimiter)| {
            let inner_start = index + delimiter.len_utf8();
            let inner_target = &target[inner_start..];
            let inner = match delimiter {
                '#' => InnerSelector::Heading(inner_target),
                '@' => InnerSelector::Id(inner_target),
                _ => unreachable!("filtered link target delimiter"),
            };
            (&target[..index], Some(inner))
        });

        let document = if document.is_empty() && inner.is_some() {
            DocumentSelector::Current
        } else if let Some(target) = document.strip_prefix('/') {
            DocumentSelector::Root(target)
        } else if let Some(target) = document.strip_prefix('+') {
            DocumentSelector::Child(target)
        } else {
            DocumentSelector::Legacy(document)
        };

        Self { document, inner }
    }
}

pub fn parse_note_link_target(target: &str) -> NoteLinkTarget<'_> {
    NoteLinkTarget::parse(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_document_and_inner_selector_combinations() {
        assert_eq!(
            NoteLinkTarget::parse("#heading"),
            NoteLinkTarget {
                document: DocumentSelector::Current,
                inner: Some(InnerSelector::Heading("heading")),
            }
        );
        assert_eq!(
            NoteLinkTarget::parse("@block-id"),
            NoteLinkTarget {
                document: DocumentSelector::Current,
                inner: Some(InnerSelector::Id("block-id")),
            }
        );
        assert_eq!(
            NoteLinkTarget::parse("/plans/future#coding"),
            NoteLinkTarget {
                document: DocumentSelector::Root("plans/future"),
                inner: Some(InnerSelector::Heading("coding")),
            }
        );
        assert_eq!(
            NoteLinkTarget::parse("+coding@week-1"),
            NoteLinkTarget {
                document: DocumentSelector::Child("coding"),
                inner: Some(InnerSelector::Id("week-1")),
            }
        );
        assert_eq!(
            NoteLinkTarget::parse("legacy"),
            NoteLinkTarget {
                document: DocumentSelector::Legacy("legacy"),
                inner: None,
            }
        );
    }

    #[test]
    fn uses_the_earliest_delimiter_and_preserves_the_raw_remainder() {
        assert_eq!(
            NoteLinkTarget::parse("/note#heading@work"),
            NoteLinkTarget {
                document: DocumentSelector::Root("note"),
                inner: Some(InnerSelector::Heading("heading@work")),
            }
        );
        assert_eq!(
            NoteLinkTarget::parse("+note@id#part"),
            NoteLinkTarget {
                document: DocumentSelector::Child("note"),
                inner: Some(InnerSelector::Id("id#part")),
            }
        );
    }

    #[test]
    fn keeps_empty_document_targets_distinct_from_current_document_selectors() {
        assert_eq!(
            NoteLinkTarget::parse(""),
            NoteLinkTarget {
                document: DocumentSelector::Legacy(""),
                inner: None,
            }
        );
        assert_eq!(
            NoteLinkTarget::parse("/"),
            NoteLinkTarget {
                document: DocumentSelector::Root(""),
                inner: None,
            }
        );
        assert_eq!(
            NoteLinkTarget::parse("+@id"),
            NoteLinkTarget {
                document: DocumentSelector::Child(""),
                inner: Some(InnerSelector::Id("id")),
            }
        );
    }
}
