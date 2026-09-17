use bridge_transfer::error::TransferError;
use bridge_transfer::manifest::{create_manifest_from_bytes, create_manifest_from_file};
use bridge_transfer::message::{TransferMessage, CHUNK_SIZE};
use bridge_transfer::receiver::FileReceiver;
use bridge_transfer::sender::FileSender;
use std::path::PathBuf;

fn temp_test_dir(sub: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bridgeos_test_transfer")
        .join(sub);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[tokio::test]
async fn test_small_file_memory_transfer() {
    let data = b"BridgeOS small payload transfer testing".to_vec();
    let manifest = create_manifest_from_bytes("small1", "test.txt", &data);

    assert_eq!(manifest.total_chunks, 1);
    assert_eq!(manifest.total_size, data.len() as u64);

    let mut sender = FileSender::from_bytes(data.clone(), manifest.clone(), 0).expect("sender");
    let mut receiver = FileReceiver::to_memory(manifest);

    let chunk = sender
        .next_chunk()
        .await
        .expect("read chunk")
        .expect("some chunk");
    assert_eq!(chunk.chunk_index, 0);
    assert_eq!(chunk.offset, 0);
    assert!(chunk.verify_hash());

    receiver.receive_chunk(&chunk).await.expect("receive chunk");
    assert!(receiver.is_complete());

    let final_bytes = receiver.finalize_memory().expect("finalize");
    assert_eq!(final_bytes, data);
}

#[tokio::test]
async fn test_multi_chunk_file_roundtrip() {
    let test_dir = temp_test_dir("multi_chunk");
    let source_path = test_dir.join("source.bin");
    let dest_dir = test_dir.join("dest");

    // 200,000 bytes spanning 4 chunks (64KB + 64KB + 64KB + 8064B)
    let payload_size = 200_000usize;
    let mut original_data = vec![0u8; payload_size];
    for (i, byte) in original_data.iter_mut().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let val = (i % 256) as u8;
        *byte = val;
    }

    tokio::fs::write(&source_path, &original_data)
        .await
        .expect("write source");

    let manifest = create_manifest_from_file(&source_path)
        .await
        .expect("manifest");
    assert_eq!(manifest.total_chunks, 4);
    assert_eq!(manifest.total_size, payload_size as u64);

    let mut sender = FileSender::from_file(&source_path, manifest.clone(), 0)
        .await
        .expect("sender");
    let mut receiver = FileReceiver::to_dir(&dest_dir, manifest.clone())
        .await
        .expect("receiver");

    while let Some(chunk) = sender.next_chunk().await.expect("read") {
        receiver.receive_chunk(&chunk).await.expect("receive");
    }

    assert!(receiver.is_complete());
    let final_path = receiver.finalize_file().await.expect("finalize");

    assert_eq!(final_path, dest_dir.join(&manifest.filename));
    let written = tokio::fs::read(&final_path).await.expect("read written");
    assert_eq!(written, original_data);

    // Verify .part file is gone
    let part_path = dest_dir.join(format!("{}.part", manifest.filename));
    assert!(!part_path.exists());
}

#[tokio::test]
async fn test_resumable_transfer_after_interruption() {
    let test_dir = temp_test_dir("resumption");
    let source_path = test_dir.join("interrupted_source.bin");
    let dest_dir = test_dir.join("dest");

    // 192KB (exactly 3 chunks of 64KB)
    let total_size = 3 * CHUNK_SIZE;
    let mut data = vec![0u8; total_size];
    for (i, byte) in data.iter_mut().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let val = ((i * 7) % 251) as u8;
        *byte = val;
    }

    tokio::fs::write(&source_path, &data)
        .await
        .expect("write source");
    let manifest = create_manifest_from_file(&source_path)
        .await
        .expect("manifest");
    assert_eq!(manifest.total_chunks, 3);

    // 1. First session: send only 1 chunk, then simulate network drop
    {
        let mut sender = FileSender::from_file(&source_path, manifest.clone(), 0)
            .await
            .expect("sender");
        let mut receiver = FileReceiver::to_dir(&dest_dir, manifest.clone())
            .await
            .expect("receiver");

        let chunk0 = sender.next_chunk().await.expect("next").expect("chunk 0");
        assert_eq!(chunk0.chunk_index, 0);
        receiver
            .receive_chunk(&chunk0)
            .await
            .expect("write chunk 0");

        assert_eq!(receiver.next_expected_chunk(), 1);
        assert!(!receiver.is_complete());
        // Receiver dropped here without finalization
    }

    // Check .part file exists on disk with exactly 1 chunk (64KB)
    let part_path = dest_dir.join(format!("{}.part", manifest.filename));
    assert!(part_path.exists());
    let part_len = tokio::fs::metadata(&part_path)
        .await
        .expect("metadata")
        .len();
    assert_eq!(part_len, CHUNK_SIZE as u64);

    // 2. Second session: resume transfer from last verified chunk offset
    {
        let mut receiver = FileReceiver::to_dir(&dest_dir, manifest.clone())
            .await
            .expect("resume receiver");
        let resume_chunk = receiver.next_expected_chunk();
        assert_eq!(resume_chunk, 1, "Should resume starting at chunk index 1");

        let mut sender = FileSender::from_file(&source_path, manifest.clone(), resume_chunk)
            .await
            .expect("resume sender");
        assert_eq!(sender.current_chunk(), 1);

        while let Some(chunk) = sender.next_chunk().await.expect("next") {
            receiver.receive_chunk(&chunk).await.expect("receive chunk");
        }

        assert!(receiver.is_complete());
        let final_path = receiver.finalize_file().await.expect("finalize");
        let received_data = tokio::fs::read(&final_path).await.expect("read final");
        assert_eq!(received_data, data);
    }
}

#[tokio::test]
async fn test_corrupted_chunk_rejection() {
    let data = b"BridgeOS Corrupted Chunk Test Data".to_vec();
    let manifest = create_manifest_from_bytes("corrupt1", "corrupt.txt", &data);

    let mut sender = FileSender::from_bytes(data, manifest.clone(), 0).expect("sender");
    let mut receiver = FileReceiver::to_memory(manifest);

    let mut chunk = sender.next_chunk().await.expect("next").expect("chunk");
    // Corrupt payload data
    chunk.data[0] ^= 0xFF;

    let res = receiver.receive_chunk(&chunk).await;
    match res {
        Err(TransferError::ChunkHashMismatch { index, .. }) => {
            assert_eq!(index, 0);
        }
        other => panic!("Expected ChunkHashMismatch error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_corrupted_root_hash_detection() {
    let data = b"BridgeOS Root Hash Validation".to_vec();
    let mut manifest = create_manifest_from_bytes("roothash1", "file.txt", &data);
    // Forged root hash
    manifest.blake3_root_hash[0] ^= 0xFF;

    let mut sender = FileSender::from_bytes(data, manifest.clone(), 0).expect("sender");
    let mut receiver = FileReceiver::to_memory(manifest);

    let chunk = sender.next_chunk().await.expect("read").expect("chunk");
    // Chunk hash is valid
    assert!(chunk.verify_hash());
    receiver
        .receive_chunk(&chunk)
        .await
        .expect("chunk accepted");

    // But whole-file finalization must detect root mismatch
    let res = receiver.finalize_memory();
    match res {
        Err(TransferError::RootHashMismatch { .. }) => {}
        other => panic!("Expected RootHashMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn test_empty_file_transfer() {
    let data = Vec::<u8>::new();
    let manifest = create_manifest_from_bytes("empty1", "empty.txt", &data);
    assert_eq!(manifest.total_chunks, 0);
    assert_eq!(manifest.total_size, 0);

    let mut sender = FileSender::from_bytes(data.clone(), manifest.clone(), 0).expect("sender");
    let receiver = FileReceiver::to_memory(manifest);

    let next = sender.next_chunk().await.expect("read");
    assert!(next.is_none());
    assert!(receiver.is_complete());

    let finalized = receiver.finalize_memory().expect("finalize");
    assert_eq!(finalized, data);
}

#[tokio::test]
async fn test_wire_transfer_message_data_frame_roundtrip() {
    let data = b"Postcard serialization wire test".to_vec();
    let manifest = create_manifest_from_bytes("wire1", "wire.bin", &data);

    let offer = TransferMessage::Offer(manifest.clone());
    let data_frame = offer.to_data_frame().expect("to data frame");
    let decoded = TransferMessage::from_data_frame(&data_frame).expect("from data frame");
    assert_eq!(decoded, offer);

    let accept = TransferMessage::Accept {
        file_id: "wire1".to_string(),
        start_chunk: 3,
    };
    let data_frame = accept.to_data_frame().expect("to data frame");
    let decoded = TransferMessage::from_data_frame(&data_frame).expect("from data frame");
    assert_eq!(decoded, accept);

    let chunk = bridge_transfer::message::FileChunk::new(
        "wire1",
        3,
        3 * (CHUNK_SIZE as u64),
        vec![1, 2, 3],
    );
    let chunk_msg = TransferMessage::Data(chunk);
    let data_frame = chunk_msg.to_data_frame().expect("to data frame");
    let decoded = TransferMessage::from_data_frame(&data_frame).expect("from data frame");
    assert_eq!(decoded, chunk_msg);
}
