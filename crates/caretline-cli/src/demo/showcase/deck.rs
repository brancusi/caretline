//! The presentation is data: copy, annotations and timed engine operations.

use super::Action;

pub(super) struct Note {
    pub anchor: &'static str,
    pub title: &'static str,
    pub text: &'static str,
    pub at: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Art {
    None,
    Typography,
    Ascii,
    Motion,
    Finale,
}

pub(super) struct Slide {
    pub title: &'static str,
    pub text: &'static str,
    pub outline: bool,
    pub art: Art,
    pub notes: &'static [Note],
    pub rings: &'static [&'static str],
    pub cues: &'static [(u8, Action)],
}

pub(super) const SLIDES: &[Slide] = &[
    Slide {
        title: "Meet Caretline",
        outline: false,
        art: Art::None,
        text: "# CARETLINE\n\nThe editor is a value.\n\nText. Carets. History. Marks. Folds. Views.\n\n\n\n\n\n\n\n\n\nA pure engine, not a terminal widget.\nOne state + messages = a replayable editor.\n\nThis is a real editor frame, not a slideshow image.\nEverything runs locally. No agent or network service needed.\n",
        notes: &[
            Note {
                anchor: "editor is a value",
                title: "An editor you can drive",
                text: "Embed it. Send messages. Save its state. Replay exactly.",
                at: 0,
            },
            Note {
                anchor: "pure engine",
                title: "Your host, your interface",
                text: "The host supplies files, a terminal, commands and overlays. The engine stays pure.",
                at: 45,
            },
        ],
        rings: &["Text.", "Carets.", "History."],
        cues: &[],
    },
    Slide {
        title: "Live message stream",
        outline: false,
        art: Art::None,
        text: "# LIVE INPUT\n\nMessages become edits, immediately.\n\nMessage stream: \n\n\n\n\n\n\n\n\n\nThe script sends InsertText through the real session.\nWrapping, carets and undo use the same engine as your app.\nResize this window while the demo runs.\n",
        notes: &[Note {
            anchor: "Message stream:",
            title: "Updates, not screenshots",
            text: "Each timed chunk is a real editor message. The caret follows the incoming text.",
            at: 0,
        }],
        rings: &[],
        cues: &[
            (8, Action::Caret("Message stream: ")),
            (15, Action::Type("Hello")),
            (25, Action::Type(", Caretline.")),
            (40, Action::Type(" Messages arrive")),
            (55, Action::Type(" while the UI stays live.")),
            (70, Action::Type(" Unicode too: cafe\u{301}, 東京, 🦀.")),
            (88, Action::CheckStream),
        ],
    },
    Slide {
        title: "Many carets, exact undo",
        outline: false,
        art: Art::None,
        text: "# ONE EDIT, THREE CARETS\n\nThree independent insertion points.\n\n- alpha\n- beta\n- gamma\n\n\n\n\n\n\n\n\nOne InsertText edits all three rows atomically.\nUndo and redo restore text AND the selection.\n",
        notes: &[Note {
            anchor: "alpha",
            title: "An edit is one transaction",
            text: "Watch three carets insert ready at once. Then undo. Then redo, with every caret restored.",
            at: 0,
        }],
        rings: &["beta", "gamma"],
        cues: &[
            (12, Action::Carets(&["alpha", "beta", "gamma"])),
            (30, Action::Type("ready ")),
            (45, Action::CheckMulti),
            (58, Action::UndoMulti),
            (78, Action::RedoMulti),
        ],
    },
    Slide {
        title: "Overlay choreography",
        outline: false,
        art: Art::None,
        text: "# MULTIPLE POINTS OF INTEREST\n\nLATENCY     12 ms\n\nMEMORY      48 MB\n\nDELIVERY    ready\n\n\n\n\n\n\n\n\nThree rings. Two callouts. One multi-row spotlight.\nLayers are anchored to text, not fixed screen coordinates.\nScroll with Up / Down; resize to see placement adapt.\n",
        notes: &[
            Note {
                anchor: "LATENCY",
                title: "Follow the signal",
                text: "A ring, an arrow and a callout, placed from a text anchor.",
                at: 0,
            },
            Note {
                anchor: "DELIVERY",
                title: "More than one highlight",
                text: "Independent layers coexist. The spotlight has holes across several rows.",
                at: 35,
            },
        ],
        rings: &["12 ms", "48 MB", "ready"],
        cues: &[
            (18, Action::Highlights(&["12 ms", "48 MB", "ready"])),
            (75, Action::CheckHighlights),
        ],
    },
    Slide {
        title: "One document, two views",
        outline: false,
        art: Art::None,
        text: "# TWO VIEWS, ONE DOCUMENT\n\nA pane is a view, not a copy.\n\nShared note: \n\n\n\n\n\n\n\n\n\nBoth panes see the same underlying text.\nEach view owns its caret, selection, folds and scroll.\nExternal edits map every view through the change.\n",
        notes: &[Note {
            anchor: "Shared note:",
            title: "Write once, see it twice",
            text: "The second pane has its own caret. No duplicated document or synchronization glue.",
            at: 0,
        }],
        rings: &[],
        cues: &[
            (10, Action::Caret("Shared note: ")),
            (20, Action::OpenView),
            (40, Action::Type("hello from view A")),
            (
                65,
                Action::RemoteAppend("Shared note:", " + an external update"),
            ),
            (85, Action::CheckViews),
        ],
    },
    Slide {
        title: "Safe co-editing",
        outline: false,
        art: Art::None,
        text: "# LOCAL + REMOTE\n\nPerson: \n\nAgent: \n\nGuard: waiting\nUndo: waiting\n\n\n\n\n\n\n\nGuard writes with if_rev. Refuse stale work, then retry.\nUndo removes your edits, not someone else's.\nA real socket is listening throughout this showcase.\n",
        notes: &[
            Note {
                anchor: "Guard:",
                title: "No silent overwrite",
                text: "The script tries a genuinely stale protocol write. It must be refused before retrying.",
                at: 0,
            },
            Note {
                anchor: "Agent:",
                title: "Undo stays yours",
                text: "The remote text is an External change. Undo must keep it while removing the local edit.",
                at: 50,
            },
        ],
        rings: &[],
        cues: &[
            (10, Action::Caret("Person: ")),
            (20, Action::RememberRev),
            (30, Action::Type("my local edit")),
            (45, Action::GuardedRemote),
            (70, Action::UndoLocal),
            (88, Action::RedoLocal),
        ],
    },
    Slide {
        title: "Structure with identity",
        outline: true,
        art: Art::None,
        text: "# FOLDS, MOVES, STABLE MARKS\n\nA block is more than a row number.\n\n- Explore the engine\n  - Carets and selections\n  - Undo and redo\n  - Views and layouts\n- Embed it in your host\n\n\n\n\n\n\n\nFold children. Reopen. Move the whole subtree.\nThe block's mark remains the same across each operation.\n",
        notes: &[Note {
            anchor: "Explore the engine",
            title: "Identity survives movement",
            text: "This item's children fold together and move together. Watch the mark verification.",
            at: 0,
        }],
        rings: &[],
        cues: &[
            (10, Action::RememberMark),
            (25, Action::Fold),
            (42, Action::Unfold),
            (60, Action::MoveBlock),
            (82, Action::CheckMark),
        ],
    },
    Slide {
        title: "Save it. Replay it. Prove it.",
        outline: false,
        art: Art::None,
        text: "# ONE STATE, EXACT REPLAY\n\nSerializable editor state, including undo history.\n\nEvidence: \n\nState round-trip: pending\nTrace replay: pending\n\n\n\n\n\n\n\n\nNext: caretline notes.md --listen\nEmbed: cargo add caretline\nControl: caretline send --latest state.get\n\nHome restarts the showcase. Left revisits any slide.\n",
        notes: &[
            Note {
                anchor: "State round-trip:",
                title: "Not just the text",
                text: "Serialize and load the real editor state. Compare the entire value, history included.",
                at: 0,
            },
            Note {
                anchor: "Trace replay:",
                title: "Check the promise",
                text: "Replay the real message trace, then compare states byte-for-byte. These are live checks.",
                at: 45,
            },
        ],
        rings: &[],
        cues: &[
            (10, Action::Caret("Evidence: ")),
            (20, Action::Type("state + messages = reproducible")),
            (40, Action::RoundTrip),
            (65, Action::ReplayCheck),
            (90, Action::Summary),
        ],
    },
    Slide {
        title: "Typography beyond the grid",
        outline: false,
        art: Art::Typography,
        text: "# TYPE AS A GRAPHICS LAYER\n\nGeist Mono + Geist / three sizes / weight 500\n\n\nGEIST MONO                  GEIST\nState / small               State / small\n\nState / medium              State / medium\n\nState / large               State / large\n\n\n\n\n\n\n\n\n\n\n\nPixel mode renders the real bundled font outlines.\nTerminal text still uses Ghostty's configured font and size.\nPress p to compare: cells cannot mix arbitrary font sizes.\n",
        notes: &[],
        rings: &[],
        cues: &[],
    },
    Slide {
        title: "ASCII is live editor state",
        outline: false,
        art: Art::Ascii,
        text: "# ASCII IN THE EDITOR\n\nTimed frame requests replace text and selected highlights.\n",
        notes: &[],
        rings: &[],
        cues: &[],
    },
    Slide {
        title: "Motion between the cells",
        outline: false,
        art: Art::Motion,
        text: "# FRACTIONAL-PIXEL MOTION\n\nA device-pixel layer, composited over the terminal.\n\n\nA moving caret and a Bezier path.\nFractional coordinates, anti-aliasing, one selection colour.\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\nPixels: continuous movement inside a fixed cell grid.\nCells: the explanatory text remains available.\nSpace freezes motion. p compares renderers. t changes transport.\n",
        notes: &[],
        rings: &[],
        cues: &[],
    },
    Slide {
        title: "caretline / the selection is the logo",
        outline: false,
        art: Art::Finale,
        text: "# CARETLINE FINALE\n\nThe original warp-field logo, live in the real editor.\n",
        notes: &[],
        rings: &[],
        cues: &[],
    },
];
