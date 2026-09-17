use bridge_integration_tests::init_test_logging;
use bridge_protocol::Frame;
use bridge_transfer::manifest::create_manifest_from_file;
use bridge_transfer::message::{TransferMessage, CHUNK_SIZE};
use bridge_transfer::receiver::FileReceiver;
use bridge_transfer::sender::FileSender;
use bridge_transport::FramedStream;
use std::path::PathBuf;
use tokio::net::{TcpListener, TcpStream};
use tracing::info;

fn temp_test_dir(sub: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bridgeos_integration_transfer")
        .join(sub);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[tokio::test]
async fn test_milestone2_tcp_file_streaming_roundtrip() {
    init_test_logging();
    info!("Starting Milestone 2 TCP file streaming test");

    let test_dir = temp_test_dir("tcp_roundtrip");
    let source_path = test_dir.join("payload.bin");
    let dest_dir = test_dir.join("received");

    // 150KB spans 3 chunks (64KB, 64KB, 22KB)
    let payload_size = 150_000usize;
    let mut original_data = vec![0u8; payload_size];
    for (i, byte) in original_data.iter_mut().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let val = ((i * 13) % 255) as u8;
        *byte = val;
    }
    tokio::fs::write(&source_path, &original_data)
        .await
        .expect("write test file");

    let manifest = create_manifest_from_file(&source_path)
        .await
        .expect("manifest");
    assert_eq!(manifest.total_chunks, 3);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");

    let server_dest = dest_dir.clone();
    let server_manifest = manifest.clone();

    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept");
        let mut server = FramedStream::new(socket);

        // 1. Await Offer
        let frame = server.recv_frame().await.unwrap().expect("offer frame");
        let offer_msg = match frame {
            Frame::Data(df) => TransferMessage::from_data_frame(&df).expect("msg"),
            other => panic!("Expected Data frame, got {other:?}"),
        };

        let file_manifest = match offer_msg {
            TransferMessage::Offer(m) => m,
            other => panic!("Expected Offer, got {other:?}"),
        };
        assert_eq!(file_manifest.file_id, server_manifest.file_id);

        let mut receiver = FileReceiver::to_dir(&server_dest, file_manifest.clone())
            .await
            .expect("receiver");

        // 2. Send Accept
        let accept = TransferMessage::Accept {
            file_id: file_manifest.file_id.clone(),
            start_chunk: receiver.next_expected_chunk(),
        };
        server
            .send_frame(&Frame::Data(accept.to_data_frame().expect("frame")))
            .await
            .expect("send accept");

        // 3. Receive Chunks
        loop {
            let frame = server.recv_frame().await.unwrap().expect("chunk frame");
            let msg = match frame {
                Frame::Data(df) => TransferMessage::from_data_frame(&df).expect("msg"),
                other => panic!("Expected Data frame, got {other:?}"),
            };

            match msg {
                TransferMessage::Data(chunk) => {
                    let idx = chunk.chunk_index;
                    receiver.receive_chunk(&chunk).await.expect("write chunk");
                    let ack = TransferMessage::Ack {
                        file_id: file_manifest.file_id.clone(),
                        chunk_index: idx,
                    };
                    server
                        .send_frame(&Frame::Data(ack.to_data_frame().expect("ack frame")))
                        .await
                        .expect("send ack");
                }
                TransferMessage::Finished {
                    file_id,
                    blake3_root_hash,
                } => {
                    assert_eq!(file_id, file_manifest.file_id);
                    assert_eq!(blake3_root_hash, file_manifest.blake3_root_hash);
                    break;
                }
                other => panic!("Unexpected msg: {other:?}"),
            }
        }

        assert!(receiver.is_complete());
        let finalized_path = receiver.finalize_file().await.expect("finalize");

        // 4. Send Complete
        let complete = TransferMessage::Complete {
            file_id: file_manifest.file_id.clone(),
        };
        server
            .send_frame(&Frame::Data(complete.to_data_frame().expect("comp frame")))
            .await
            .expect("send complete");

        finalized_path
    });

    // Client execution
    let stream = TcpStream::connect(addr).await.expect("connect");
    let mut client = FramedStream::new(stream);

    // 1. Send Offer
    let offer = TransferMessage::Offer(manifest.clone());
    client
        .send_frame(&Frame::Data(offer.to_data_frame().expect("offer frame")))
        .await
        .expect("send offer");

    // 2. Receive Accept
    let accept_frame = client.recv_frame().await.unwrap().expect("accept frame");
    let start_chunk = match accept_frame {
        Frame::Data(df) => match TransferMessage::from_data_frame(&df).expect("msg") {
            TransferMessage::Accept { start_chunk, .. } => start_chunk,
            other => panic!("Expected Accept, got {other:?}"),
        },
        other => panic!("Expected Data frame, got {other:?}"),
    };
    assert_eq!(start_chunk, 0);

    // 3. Stream Chunks
    let mut sender = FileSender::from_file(&source_path, manifest.clone(), start_chunk)
        .await
        .expect("sender");

    while let Some(chunk) = sender.next_chunk().await.expect("next chunk") {
        let chunk_msg = TransferMessage::Data(chunk);
        client
            .send_frame(&Frame::Data(
                chunk_msg.to_data_frame().expect("chunk frame"),
            ))
            .await
            .expect("send chunk");

        // Await Ack
        let ack_frame = client.recv_frame().await.unwrap().expect("ack frame");
        match ack_frame {
            Frame::Data(df) => match TransferMessage::from_data_frame(&df).expect("ack") {
                TransferMessage::Ack { chunk_index, .. } => {
                    assert_eq!(chunk_index, sender.current_chunk() - 1);
                }
                other => panic!("Expected Ack, got {other:?}"),
            },
            other => panic!("Expected Data frame, got {other:?}"),
        }
    }

    // 4. Send Finished
    let finished = TransferMessage::Finished {
        file_id: manifest.file_id.clone(),
        blake3_root_hash: manifest.blake3_root_hash,
    };
    client
        .send_frame(&Frame::Data(
            finished.to_data_frame().expect("finish frame"),
        ))
        .await
        .expect("send finished");

    // 5. Await Complete
    let comp_frame = client.recv_frame().await.unwrap().expect("comp frame");
    match comp_frame {
        Frame::Data(df) => match TransferMessage::from_data_frame(&df).expect("complete") {
            TransferMessage::Complete { file_id } => {
                assert_eq!(file_id, manifest.file_id);
            }
            other => panic!("Expected Complete, got {other:?}"),
        },
        other => panic!("Expected Data frame, got {other:?}"),
    }

    let saved_file = server_task.await.expect("server join");
    let written = tokio::fs::read(&saved_file).await.expect("read saved");
    assert_eq!(written, original_data);
    info!("Milestone 2 TCP file streaming test passed successfully!");
}

#[tokio::test]
async fn test_milestone2_resumable_transfer_over_tcp_network_failure() {
    init_test_logging();
    info!("Starting Milestone 2 TCP resumable transfer test across network drops");

    let test_dir = temp_test_dir("tcp_resumption");
    let source_path = test_dir.join("resumable_source.bin");
    let dest_dir = test_dir.join("dest");

    // Exactly 3 chunks (192KB)
    let total_size = 3 * CHUNK_SIZE;
    let mut data = vec![0u8; total_size];
    for (i, byte) in data.iter_mut().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let val = ((i * 17) % 251) as u8;
        *byte = val;
    }
    tokio::fs::write(&source_path, &data)
        .await
        .expect("write source");

    let manifest = create_manifest_from_file(&source_path)
        .await
        .expect("manifest");
    assert_eq!(manifest.total_chunks, 3);

    // --- SESSION 1: Transfer chunk 0 and abruptly drop connection ---
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let dest = dest_dir.clone();

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept");
            let mut server = FramedStream::new(socket);

            // Read Offer
            let frame = server.recv_frame().await.unwrap().expect("frame");
            let offer = match frame {
                Frame::Data(df) => TransferMessage::from_data_frame(&df).expect("msg"),
                other => panic!("Expected Data, got {other:?}"),
            };
            let f_manifest = match offer {
                TransferMessage::Offer(m) => m,
                other => panic!("Expected Offer, got {other:?}"),
            };

            let mut receiver = FileReceiver::to_dir(&dest, f_manifest.clone())
                .await
                .expect("receiver");

            // Send Accept
            let accept = TransferMessage::Accept {
                file_id: f_manifest.file_id.clone(),
                start_chunk: receiver.next_expected_chunk(),
            };
            server
                .send_frame(&Frame::Data(accept.to_data_frame().unwrap()))
                .await
                .unwrap();

            // Receive exactly 1 chunk
            let chunk_frame = server.recv_frame().await.unwrap().expect("chunk");
            let chunk_msg = match chunk_frame {
                Frame::Data(df) => TransferMessage::from_data_frame(&df).unwrap(),
                other => panic!("Expected Data, got {other:?}"),
            };
            let chunk = match chunk_msg {
                TransferMessage::Data(c) => c,
                other => panic!("Expected Data, got {other:?}"),
            };
            receiver.receive_chunk(&chunk).await.unwrap();

            // Socket is closed here without finalization
        });

        let stream = TcpStream::connect(addr).await.expect("connect");
        let mut client = FramedStream::new(stream);

        let offer = TransferMessage::Offer(manifest.clone());
        client
            .send_frame(&Frame::Data(offer.to_data_frame().unwrap()))
            .await
            .unwrap();

        let _accept = client.recv_frame().await.unwrap().expect("accept");

        let mut sender = FileSender::from_file(&source_path, manifest.clone(), 0)
            .await
            .unwrap();
        let chunk0 = sender.next_chunk().await.unwrap().expect("chunk 0");
        client
            .send_frame(&Frame::Data(
                TransferMessage::Data(chunk0).to_data_frame().unwrap(),
            ))
            .await
            .unwrap();

        // Drop client abruptly
        drop(client);
        server_task.await.unwrap();
    }

    // Verify .part file exists on receiver side
    let part_path = dest_dir.join(format!("{}.part", manifest.filename));
    assert!(part_path.exists());
    assert_eq!(
        tokio::fs::metadata(&part_path).await.unwrap().len(),
        CHUNK_SIZE as u64
    );

    // --- SESSION 2: Resume transfer from last verified chunk offset (chunk 1) ---
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let dest = dest_dir.clone();

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept");
            let mut server = FramedStream::new(socket);

            // Read Offer
            let frame = server.recv_frame().await.unwrap().expect("frame");
            let offer = match frame {
                Frame::Data(df) => TransferMessage::from_data_frame(&df).expect("msg"),
                other => panic!("Expected Data, got {other:?}"),
            };
            let f_manifest = match offer {
                TransferMessage::Offer(m) => m,
                other => panic!("Expected Offer, got {other:?}"),
            };

            let mut receiver = FileReceiver::to_dir(&dest, f_manifest.clone())
                .await
                .expect("receiver");

            let resume_offset = receiver.next_expected_chunk();
            assert_eq!(resume_offset, 1, "Should detect chunk 0 already written");

            // Send Accept with resume offset
            let accept = TransferMessage::Accept {
                file_id: f_manifest.file_id.clone(),
                start_chunk: resume_offset,
            };
            server
                .send_frame(&Frame::Data(accept.to_data_frame().unwrap()))
                .await
                .unwrap();

            // Receive remaining chunks
            while let Some(frame) = server.recv_frame().await.unwrap() {
                let msg = match frame {
                    Frame::Data(df) => TransferMessage::from_data_frame(&df).unwrap(),
                    other => panic!("Expected Data, got {other:?}"),
                };
                match msg {
                    TransferMessage::Data(chunk) => {
                        receiver.receive_chunk(&chunk).await.unwrap();
                    }
                    TransferMessage::Finished { .. } => break,
                    other => panic!("Unexpected msg {other:?}"),
                }
            }

            assert!(receiver.is_complete());
            let final_path = receiver.finalize_file().await.unwrap();

            let comp = TransferMessage::Complete {
                file_id: f_manifest.file_id,
            };
            server
                .send_frame(&Frame::Data(comp.to_data_frame().unwrap()))
                .await
                .unwrap();

            final_path
        });

        let stream = TcpStream::connect(addr).await.expect("connect");
        let mut client = FramedStream::new(stream);

        // Send Offer
        let offer = TransferMessage::Offer(manifest.clone());
        client
            .send_frame(&Frame::Data(offer.to_data_frame().unwrap()))
            .await
            .unwrap();

        // Read Accept
        let accept_frame = client.recv_frame().await.unwrap().expect("accept");
        let start_chunk = match accept_frame {
            Frame::Data(df) => match TransferMessage::from_data_frame(&df).unwrap() {
                TransferMessage::Accept { start_chunk, .. } => start_chunk,
                other => panic!("Expected Accept, got {other:?}"),
            },
            other => panic!("Expected Data, got {other:?}"),
        };
        assert_eq!(start_chunk, 1, "Server negotiated resume from chunk 1");

        // Send remaining chunks starting from chunk 1
        let mut sender = FileSender::from_file(&source_path, manifest.clone(), start_chunk)
            .await
            .unwrap();
        assert_eq!(sender.current_chunk(), 1);

        while let Some(chunk) = sender.next_chunk().await.unwrap() {
            client
                .send_frame(&Frame::Data(
                    TransferMessage::Data(chunk).to_data_frame().unwrap(),
                ))
                .await
                .unwrap();
        }

        let finished = TransferMessage::Finished {
            file_id: manifest.file_id.clone(),
            blake3_root_hash: manifest.blake3_root_hash,
        };
        client
            .send_frame(&Frame::Data(finished.to_data_frame().unwrap()))
            .await
            .unwrap();

        let comp_frame = client.recv_frame().await.unwrap().expect("comp");
        match comp_frame {
            Frame::Data(df) => match TransferMessage::from_data_frame(&df).unwrap() {
                TransferMessage::Complete { file_id } => {
                    assert_eq!(file_id, manifest.file_id);
                }
                other => panic!("Expected Complete, got {other:?}"),
            },
            other => panic!("Expected Data, got {other:?}"),
        }

        let final_path = server_task.await.unwrap();
        let final_data = tokio::fs::read(&final_path).await.unwrap();
        assert_eq!(final_data, data);
        assert!(!part_path.exists());
    }

    info!("Milestone 2 resumable transfer over TCP passed successfully!");
}
