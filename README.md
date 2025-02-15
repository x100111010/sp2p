# sp2p

Simple tool for querying Spectre nodes on the p2p port.

## Build and run

Protobuf is required, see 'Installation' here: https://github.com/spectre-project/rusty-spectre

```bash
cargo run -- -s 127.0.0.1:18111 version
```

## Usage

```text
Usage: sp2p [OPTIONS] <REQUEST>

Arguments:
  <REQUEST>  Request type [possible values:
    * version: Retrieves the version of the Spectre node
    * addresses: Retrieves a list of addresses from the Spectre node
    * crawl: Initiates a network crawl starting from the specified node]

Options:
  -s, --url <URL>          The ip:port of a spectred instance [default: localhost:18111]
  -n, --network <NETWORK>  The network type and suffix, e.g. 'testnet-11' [default: mainnet]
  -o, --output <OUTPUT>    Output JSON file for crawl mode [default: nodes.json]
```

## Examples

- Query the version of a Spectre node on the default network: `sp2p version`
- Query the addresses of a Spectre node on a test network: `sp2p -n testnet-11 addresses`
- Query a Spectre node at a custom IP and port: `sp2p -s 127.0.0.1:18111 version`
- Start the network crawler and serve the node data: `cargo run -- crawl --api-port 3000`

### Thank You
A thank you to suprtypo@pm.me for the [original work](https://github.com/supertypo/kp2p). 
It provided the general approach used here, as well as the initial codebase.
