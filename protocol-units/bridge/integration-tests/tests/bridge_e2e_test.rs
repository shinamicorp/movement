use alloy::primitives::keccak256;
use anyhow::Result;
use bridge_integration_tests::HarnessEthClient;
use bridge_integration_tests::TestHarness;
use bridge_service::chains::bridge_contracts::BridgeContract;
use bridge_service::chains::bridge_contracts::BridgeContractEvent;
use bridge_service::chains::ethereum::event_monitoring::EthMonitoring;
use bridge_service::chains::{
	ethereum::types::EthAddress,
	movement::{event_monitoring::MovementMonitoring, utils::MovementAddress},
};
use bridge_service::types::Amount;
use bridge_service::types::AssetType;
use bridge_service::types::HashLock;
use bridge_service::types::HashLockPreImage;
use futures::StreamExt;
use tracing_subscriber::EnvFilter;

#[tokio::test]
async fn test_bridge_transfer_eth_movement_happy_path() -> Result<(), anyhow::Error> {
	tracing_subscriber::fmt()
		.with_env_filter(
			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
		)
		.init();

	let (eth_client_harness, mut mvt_client_harness, config) =
		TestHarness::new_with_eth_and_movement().await?;

	tracing::info!("Init initiator and counter part test account.");
	tracing::info!("Use client signer for Mvt and index 2 of config.eth.eth_well_known_account_private_keys array for Eth");

	// Init mvt addresses
	let movement_client_signer_address = mvt_client_harness.movement_client.signer().address();

	{
		let faucet_client = mvt_client_harness.faucet_client.write().unwrap();
		faucet_client.fund(movement_client_signer_address, 100_000_000).await?;
	}

	let recipient_privkey = mvt_client_harness.fund_account().await;
	let recipient_address = MovementAddress(recipient_privkey.address());

	// 1) initialize Eth transfer
	tracing::info!("Call initiate_transfer on Eth");
	let hash_lock_pre_image = HashLockPreImage::random();
	let hash_lock = HashLock(From::from(keccak256(hash_lock_pre_image)));
	let amount = Amount(AssetType::EthAndWeth((1, 0)));
	HarnessEthClient::initiate_eth_bridge_transfer(
		&config,
		HarnessEthClient::get_initiator_private_key(&config),
		recipient_address,
		hash_lock,
		amount,
	)
	.await
	.expect("Failed to initiate bridge transfer");

	//Wait for the tx to be executed
	tracing::info!("Wait for the MVT Locked event.");
	let mut mvt_monitoring = MovementMonitoring::build(&config.movement).await.unwrap();
	let event =
		tokio::time::timeout(std::time::Duration::from_secs(30), mvt_monitoring.next()).await?;
	let bridge_tranfer_id = if let Some(Ok(BridgeContractEvent::Locked(detail))) = event {
		detail.bridge_transfer_id
	} else {
		panic!("Not a Locked event: {event:?}");
	};

	println!("bridge_tranfer_id : {:?}", bridge_tranfer_id);
	println!("hash_lock_pre_image : {:?}", hash_lock_pre_image);

	//send counter complete event.
	tracing::info!("Call counterparty_complete_bridge_transfer on MVT.");
	let tx = mvt_client_harness
		.counterparty_complete_bridge_transfer(
			recipient_privkey,
			bridge_tranfer_id,
			hash_lock_pre_image,
		)
		.await?;

	let mut eth_monitoring = EthMonitoring::build(&config.eth).await.unwrap();
	// Wait for InitialtorCompleted event
	tracing::info!("Wait for InitialtorCompleted event.");
	loop {
		let event =
			tokio::time::timeout(std::time::Duration::from_secs(30), eth_monitoring.next()).await?;
		if let Some(Ok(BridgeContractEvent::InitialtorCompleted(_))) = event {
			break;
		}
	}

	Ok(())
}

#[tokio::test]
async fn test_bridge_transfer_movement_eth_happy_path() -> Result<(), anyhow::Error> {
	tracing_subscriber::fmt()
		.with_env_filter(
			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
		)
		.init();

	let (mut eth_client_harness, mut mvt_client_harness, config) =
		TestHarness::new_with_eth_and_movement().await?;

	let mut eth_monitoring = EthMonitoring::build(&config.eth).await.unwrap();

	mvt_client_harness.init_set_timelock(60).await?; //Set to 1mn

	tracing::info!("Init initiator and counter part test account.");

	// Init mvt addresses
	let movement_client_signer_address = mvt_client_harness.movement_client.signer().address();

	{
		let faucet_client = mvt_client_harness.faucet_client.write().unwrap();
		faucet_client.fund(movement_client_signer_address, 100_000_000_000).await?;
	}

	let recipient_privkey = mvt_client_harness.fund_account().await;
	let recipient_address = MovementAddress(recipient_privkey.address());

	let counterpart_privekey = HarnessEthClient::get_initiator_private_key(&config);
	let counter_party_address = EthAddress(counterpart_privekey.address());

	//mint initiator to have enough moveeth to do the transfer
	mvt_client_harness.mint_moveeth(&recipient_address, 1).await?;

	// 1) initialize Movement transfer
	let hash_lock_pre_image = HashLockPreImage::random();
	let hash_lock = HashLock(From::from(keccak256(hash_lock_pre_image)));
	let amount = 1;
	mvt_client_harness
		.initiate_bridge_transfer(&recipient_privkey, counter_party_address, hash_lock, amount)
		.await?;

	// Wait for InitialtorCompleted event
	tracing::info!("Wait for Bridge Lock event.");
	let bridge_tranfer_id;
	loop {
		let event =
			tokio::time::timeout(std::time::Duration::from_secs(30), eth_monitoring.next()).await?;
		if let Some(Ok(BridgeContractEvent::Locked(detail))) = event {
			bridge_tranfer_id = detail.bridge_transfer_id;
			break;
		}
	}

	// 2) Complete transfer on Eth
	eth_client_harness
		.eth_client
		.counterparty_complete_bridge_transfer(bridge_tranfer_id, hash_lock_pre_image)
		.await?;

	loop {
		let event =
			tokio::time::timeout(std::time::Duration::from_secs(30), eth_monitoring.next()).await?;
		if let Some(Ok(BridgeContractEvent::CounterPartCompleted(id, _))) = event {
			assert_eq!(bridge_tranfer_id, id);
			break;
		}
	}

	Ok(())
}

// #[tokio::test]
// async fn test_movement_event() -> Result<(), anyhow::Error> {
// 	tracing_subscriber::fmt()
// 		.with_env_filter(
// 			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
// 		)
// 		.init();

// 	println!("Start test_movement_event",);

// 	let config = TestHarness::read_bridge_config().await?;
// 	println!("after test_movement_event",);

// 	//	1) initialize transfer
// 	// let hash_lock_pre_image = HashLockPreImage::random();
// 	// let hash_lock = HashLock(From::from(keccak256(hash_lock_pre_image)));
// 	// let mov_recipient = MovementAddress(AccountAddress::new(*b"0x00000000000000000000000000face"));

// 	// let amount = Amount(AssetType::EthAndWeth((1, 0)));
// 	// initiate_eth_bridge_transfer(
// 	// 	&config,
// 	// 	HarnessEthClient::get_initiator_private_key(&config),
// 	// 	mov_recipient,
// 	// 	hash_lock,
// 	// 	amount,
// 	// )
// 	// .await
// 	// .expect("Failed to initiate bridge transfer");

// 	use bridge_integration_tests::MovementToEthCallArgs;

// 	let mut movement_client = MovementClient::new(&config.movement).await.unwrap();

// 	let args = MovementToEthCallArgs::default();
// 	// let signer_privkey = config.movement.movement_signer_address.clone();
// 	// let sender_address = format!("0x{}", Ed25519PublicKey::from(&signer_privkey).to_string());
// 	// let sender_address = movement_client.signer().address();
// 	//		test_utils::fund_and_check_balance(&mut mvt_client_harness, 100_000_000_000).await?;
// 	bridge_integration_tests::utils::initiate_bridge_transfer_helper(
// 		&mut movement_client,
// 		args.initiator.0,
// 		args.recipient.clone(),
// 		args.hash_lock.0,
// 		args.amount,
// 		true,
// 	)
// 	.await
// 	.expect("Failed to initiate bridge transfer");

// 	//Wait for the tx to be executed
// 	let _ = tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
// 	let event_type = format!(
// 		"{}::atomic_bridge_initiator::BridgeTransferStore",
// 		config.movement.movement_native_address
// 	);

// 	// let signer_privkey = config.movement.movement_signer_address.clone();
// 	// let signer_public_key = format!("0x{}", Ed25519PublicKey::from(&signer_privkey).to_string());

// 	// println!("signer_public_key {signer_public_key}",);

// 	// let res = fetch_account_events(
// 	// 	&config.movement.mvt_rpc_connection_url(),
// 	// 	&signer_public_key,
// 	// 	&event_type,
// 	// )
// 	// .await;
// 	// println!("res: {res:?}",);

// 	// let res = test_get_events_by_account_event_handle(
// 	// 	&config.movement.mvt_rpc_connection_url(),
// 	// 	"0xf90391c81027f03cdea491ed8b36ffaced26b6df208a9b569e5baf2590eb9b16",
// 	// 	&event_type,
// 	// )
// 	// .await;
// 	// println!("res: {res:?}",);

// 	let res = test_get_events_by_account_event_handle(
// 		&config.movement.mvt_rpc_connection_url(),
// 		&config.movement.movement_native_address,
// 		&event_type,
// 	)
// 	.await;
// 	println!("res: {res:?}",);

// 	let res = fetch_account_events(
// 		&config.movement.mvt_rpc_connection_url(),
// 		&config.movement.movement_native_address,
// 		&event_type,
// 	)
// 	.await;
// 	println!("res: {res:?}",);

// 	// let mut one_stream = MovementMonitoring::build(&config.movement).await?;

// 	// //listen to event.
// 	// let mut error_counter = 0;
// 	// loop {
// 	// 	tokio::select! {
// 	// 		// Wait on chain one events.
// 	// 		Some(one_event_res) = one_stream.next() =>{
// 	// 			match one_event_res {
// 	// 				Ok(one_event) => {
// 	// 					println!("Receive event {:?}", one_event);
// 	// 				}
// 	// 				Err(err) => {
// 	// 					println!("Receive error {:?}", err);
// 	// 					error_counter +=1;
// 	// 					if error_counter > 5 {
// 	// 						break;
// 	// 					}
// 	// 				}
// 	// 			}
// 	// 		}
// 	// 	}
// 	// }

// 	Ok(())
// }

// async fn test_get_events_by_account_event_handle(
// 	rest_url: &str,
// 	account_address: &str,
// 	event_type: &str,
// ) {
// 	// let url =
// 	// 	format!("{}/v1/accounts/{}/events?event_type={}", rest_url, account_address, event_type);
// 	let url = format!(
// 		"{}/v1/accounts/{}/events/{}/bridge_transfer_initiated_events",
// 		rest_url, account_address, event_type
// 	);

// 	println!("url: {:?}", url);
// 	let client = reqwest::Client::new();

// 	// Send the GET request
// 	let response = client
// 		.get(&url)
// 		.query(&[("start", "0"), ("limit", "10")])
// 		.send()
// 		.await
// 		.unwrap()
// 		.text()
// 		.await;
// 	println!("Account direct response: {response:?}",);
// }

// use aptos_sdk::rest_client::Client;
// use std::str::FromStr;

// async fn fetch_account_events(rest_url: &str, account_address: &str, event_type: &str) {
// 	// Initialize the RestClient
// 	let node_connection_url = url::Url::from_str(rest_url).unwrap();
// 	let client = Client::new(node_connection_url); // Use the correct node URL
// 	let native_address = AccountAddress::from_hex_literal(account_address).unwrap();

// 	// Get the events for the specified account
// 	let response = client
// 		.get_account_events(
// 			native_address,
// 			event_type,
// 			"bridge_transfer_initiated_events",
// 			Some(1),
// 			None,
// 		)
// 		.await;

// 	println!("response{response:?}",);

// 	// let core_code_address: AccountAddress = AccountAddress::from_hex_literal("0x1").unwrap();
// 	// let response = client
// 	// 	.get_account_events_bcs(
// 	// 		core_code_address,
// 	// 		"0x1::block::BlockResource",
// 	// 		"new_block_events",
// 	// 		Some(1),
// 	// 		None,
// 	// 	)
// 	// 	.await;
// 	// println!("new_block_events response: {response:?}",);
// }
