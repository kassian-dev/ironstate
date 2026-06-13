//! End-to-end tests for the core machine: structural enforcement, the
//! `apply`/`could_apply`/`why_not`/`peek_transition` quartet, event kinds, and
//! the `analyze!`/`test!` verification macros.

use ironstate::prelude::*;

mod article {
    use super::*;

    #[derive(StateMachine, Clone, Debug, PartialEq)]
    #[state_machine(initial = Draft, terminal = [Archived])]
    pub enum Article {
        Draft,
        Review,
        Published,
        Archived,
    }

    #[derive(Event, Clone, Debug, PartialEq)]
    pub enum Edit {
        Submit,
        Approve,
        Reject,
        Archive,
    }

    impl TransitionRules for Article {
        type Event = Edit;
        fn transition(&self, event: &Edit) -> Option<Article> {
            use Article::*;
            use Edit::*;
            match (self, event) {
                (Draft, Submit) => Some(Review),
                (Review, Approve) => Some(Published),
                (Review, Reject) => Some(Draft),
                (Published, Archive) => Some(Archived),
                _ => None,
            }
        }
    }

    ironstate::analyze!(Article);
    ironstate::test!(Article, cases = 200, max_steps = 30);

    #[test]
    fn happy_path() {
        let mut m = Machine::<Article>::new();
        assert_eq!(m.state(), &Article::Draft);
        assert_eq!(m.apply(Edit::Submit).unwrap(), Article::Review);
        assert_eq!(m.apply(Edit::Approve).unwrap(), Article::Published);
    }

    #[test]
    fn quartet_agrees() {
        let m = {
            let mut m = Machine::<Article>::new();
            m.apply(Edit::Submit).unwrap();
            m
        };
        // In Review: Approve is legal, Submit is not.
        assert!(m.could_apply(&Edit::Approve));
        assert!(m.why_not(&Edit::Approve).is_none());
        assert_eq!(m.peek_transition(&Edit::Approve), Some(Article::Published));

        assert!(!m.could_apply(&Edit::Submit));
        assert!(m.why_not(&Edit::Submit).is_some());
        assert_eq!(m.peek_transition(&Edit::Submit), None);
    }

    #[test]
    fn terminal_state_rejects_and_returns_event() {
        let mut m = Machine::restore(Article::Published);
        // Archive is the one legal move out of Published.
        assert_eq!(m.apply(Edit::Archive).unwrap(), Article::Archived);

        // Archived is terminal: every event is rejected, and the event comes back.
        let err = m.apply(Edit::Submit).unwrap_err();
        match &err {
            TransitionError::TerminalState { state, event } => {
                assert_eq!(state, &Article::Archived);
                assert_eq!(event, &Edit::Submit);
            }
            other => panic!("expected TerminalState, got {other:?}"),
        }
        assert_eq!(err.into_event(), Edit::Submit);
    }

    #[test]
    fn no_transition_is_typed() {
        let m = Machine::<Article>::new(); // Draft
        let err = m.why_not(&Edit::Approve).unwrap();
        assert!(matches!(err, TransitionError::NoTransition { .. }));
        // Display is teaching prose mentioning the state and event.
        let text = err.to_string();
        assert!(text.contains("Draft"));
        assert!(text.contains("no transition"));
    }

    #[test]
    fn listeners_fire_and_the_clock_is_injectable() {
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::time::Instant;

        let transitions = Rc::new(RefCell::new(Vec::new()));
        let rejections = Rc::new(RefCell::new(0u32));
        let fixed_time = Instant::now();

        let mut m = Machine::<Article>::new();
        m.set_clock(move || fixed_time);
        {
            let seen = Rc::clone(&transitions);
            m.on_transition(move |record| {
                seen.borrow_mut().push((
                    record.from_state.clone(),
                    record.to_state.clone(),
                    record.timestamp,
                ));
            });
            let count = Rc::clone(&rejections);
            m.on_rejection(move |_| *count.borrow_mut() += 1);
        }

        m.apply(Edit::Submit).unwrap(); // Draft -> Review: fires on_transition
        m.apply(Edit::Submit).unwrap_err(); // no transition: fires on_rejection

        let recorded = transitions.borrow();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, Article::Draft);
        assert_eq!(recorded[0].1, Article::Review);
        assert_eq!(recorded[0].2, fixed_time); // the injected clock was used
        assert_eq!(*rejections.borrow(), 1);
    }
}

mod deploy {
    use super::*;

    #[derive(StateMachine, Clone, Debug, PartialEq)]
    #[state_machine(initial = Pending, terminal = [Done])]
    pub enum Deploy {
        Pending,
        #[only_accepts(kind = "external")]
        Deploying,
        Done,
    }

    #[derive(Event, Clone, Debug, PartialEq)]
    pub enum Sig {
        Start,
        #[event_kind = "external"]
        Complete,
        Cancel,
    }

    impl TransitionRules for Deploy {
        type Event = Sig;
        fn transition(&self, event: &Sig) -> Option<Deploy> {
            use Deploy::*;
            use Sig::*;
            match (self, event) {
                (Pending, Start) => Some(Deploying),
                (Deploying, Complete) => Some(Done),
                (Pending, Cancel) => Some(Done),
                _ => None,
            }
        }
    }

    ironstate::analyze!(Deploy);
    ironstate::test!(Deploy, cases = 200, max_steps = 30);

    #[test]
    fn external_only_state_rejects_default_kind() {
        let mut m = Machine::<Deploy>::new();
        m.apply(Sig::Start).unwrap(); // Pending -> Deploying

        // Deploying accepts only `external`; a default-kind event is rejected on
        // the kind check, before the transition function is even consulted.
        let err = m.apply(Sig::Cancel).unwrap_err();
        match err {
            TransitionError::EventKindRejected {
                expected_kinds,
                event_kind,
                ..
            } => {
                assert_eq!(expected_kinds, &[Kind("external")]);
                assert_eq!(event_kind, None);
            }
            other => panic!("expected EventKindRejected, got {other:?}"),
        }

        // The matching external event is accepted.
        assert_eq!(m.apply(Sig::Complete).unwrap(), Deploy::Done);
    }
}
