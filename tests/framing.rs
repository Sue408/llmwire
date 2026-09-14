mod framing {
    use llmwire::framing::{SseFrame, SseFramer};
    use llmwire::Error;

    fn frame(event: Option<&str>, data: Option<&str>) -> SseFrame {
        SseFrame {
            event: event.map(str::to_owned),
            data: data.map(str::to_owned),
        }
    }

    #[test]
    fn splits_sse_frames_across_chunks() {
        let mut framer = SseFramer::new();
        let mut out = Vec::new();

        framer.feed(b"data: {\"x\":", &mut out).unwrap();
        assert!(out.is_empty());

        framer.feed(b"1}\n\n", &mut out).unwrap();
        assert_eq!(out, vec![frame(None, Some("{\"x\":1}"))]);
    }

    #[test]
    fn tolerates_comments_heartbeats_and_event_without_payload() {
        let mut framer = SseFramer::new();
        let mut out = Vec::new();

        framer
            .feed(
                b": keepalive\n\nevent: ping\n\ndata: first\ndata: second\n\n",
                &mut out,
            )
            .unwrap();

        assert_eq!(
            out,
            vec![
                frame(Some("ping"), None),
                frame(None, Some("first\nsecond")),
            ]
        );
    }

    #[test]
    fn accepts_crlf_frame_boundaries() {
        let mut framer = SseFramer::new();
        let mut out = Vec::new();

        framer
            .feed(b"event: message\r\ndata: hello\r\n\r\n", &mut out)
            .unwrap();

        assert_eq!(out, vec![frame(Some("message"), Some("hello"))]);
    }

    #[test]
    fn keeps_incomplete_frame_buffered() {
        let mut framer = SseFramer::new();
        let mut out = Vec::new();

        framer.feed(b"data: partial", &mut out).unwrap();

        assert!(out.is_empty());
        assert_eq!(framer.pending_len(), 13);
    }

    #[test]
    fn finish_flushes_tail_frame() {
        let mut framer = SseFramer::new();
        let mut out = Vec::new();

        framer
            .feed(b"event: message\ndata: tail", &mut out)
            .unwrap();
        framer.finish(&mut out).unwrap();

        assert_eq!(out, vec![frame(Some("message"), Some("tail"))]);
        assert_eq!(framer.pending_len(), 0);
    }

    #[test]
    fn empty_data_field_is_preserved() {
        let mut framer = SseFramer::new();
        let mut out = Vec::new();

        framer.feed(b"data:\n\n", &mut out).unwrap();

        assert_eq!(out, vec![frame(None, Some(""))]);
    }

    #[test]
    fn buffer_limit_applies_to_incomplete_frame() {
        let mut framer = SseFramer::with_limit(4);
        let mut out = Vec::new();

        let error = framer.feed(b"data: overflow", &mut out).unwrap_err();

        assert!(matches!(error, Error::BufferLimitExceeded));
        assert!(out.is_empty());
    }

    #[test]
    fn complete_frames_are_not_limited_by_total_chunk_size() {
        let mut framer = SseFramer::with_limit(8);
        let mut out = Vec::new();

        framer.feed(b"data: a\n\ndata: b\n\n", &mut out).unwrap();

        assert_eq!(out, vec![frame(None, Some("a")), frame(None, Some("b"))]);
        assert_eq!(framer.pending_len(), 0);
    }

    #[test]
    fn rejects_invalid_utf8_without_partial_output() {
        let mut framer = SseFramer::new();
        let mut out = Vec::new();

        let error = framer.feed(b"data: \xff\n\n", &mut out).unwrap_err();

        assert!(matches!(error, Error::InvalidInput(_)));
        assert!(out.is_empty());
    }
}
