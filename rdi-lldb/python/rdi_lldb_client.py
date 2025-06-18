#!/usr/bin/env python3
"""
Simple RDI-LLDB client for testing the event-driven protocol
"""

import json
import socket
import sys


class RdiClient:
    def __init__(self, host="127.0.0.1", port=9001):
        self.host = host
        self.port = port
        self.sock = None
        self.file = None

    def connect(self, binary_path):
        """Connect to server and initialize session with binary"""
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.connect((self.host, self.port))
        self.file = self.sock.makefile('rw')
        
        # Send init message
        init_msg = {
            "type": "Init",
            "binary_path": binary_path
        }
        self._send_message(init_msg)
        print(f"Connected to {self.host}:{self.port} with binary: {binary_path}")

    def _send_message(self, msg):
        """Send a message to the server"""
        msg_json = json.dumps(msg) + '\n'
        print(f">> {msg_json.strip()}")
        self.file.write(msg_json)
        self.file.flush()

    def _read_message(self):
        """Read a message from the server"""
        line = self.file.readline().strip()
        if not line:
            return None
        print(f"<< {line}")
        return json.loads(line)

    def send_command(self, cmd, args=None):
        """Send a command and handle any events"""
        if args is None:
            args = []
        
        msg = {
            "type": "Command",
            "id": 1,  # Simple static ID for testing
            "cmd": cmd,
            "args": args
        }
        
        self._send_message(msg)
        
        # Handle response/events
        while True:
            response = self._read_message()
            if not response:
                break
                
            if response.get("type") == "Event":
                # Handle event - for now just print and send dummy response
                print(f"Received event: {response}")
                # TODO: Implement proper event handling
                event_response = {
                    "type": "EventResponse",
                    "id": response["id"],
                    "event": "Error",
                    "message": "Event handling not implemented"
                }
                self._send_message(event_response)
            elif response.get("type") in ["Complete", "Error"]:
                return response
            else:
                print(f"Unknown response type: {response}")
                break

    def close(self):
        """Close the connection"""
        if self.file:
            self.file.close()
        if self.sock:
            self.sock.close()


def main():
    if len(sys.argv) < 2:
        print("Usage: python rdi_lldb_client.py <binary_path>")
        sys.exit(1)
    
    binary_path = sys.argv[1]
    client = RdiClient()
    
    try:
        client.connect(binary_path)
        
        # Interactive loop
        print("\nRDI-LLDB Test Client")
        print("Commands: eval <expr>, print <expr>, quit")
        
        while True:
            try:
                line = input("rdi> ").strip()
                if not line:
                    continue
                    
                if line == "quit" or line == "exit":
                    break
                    
                parts = line.split(' ', 1)
                cmd = parts[0]
                args = [parts[1]] if len(parts) > 1 else []
                
                response = client.send_command(cmd, args)
                if response:
                    if response.get("type") == "Complete":
                        print(f"Result: {response.get('result', {})}")
                    elif response.get("type") == "Error":
                        print(f"Error: {response.get('error', 'Unknown error')}")
                        
            except KeyboardInterrupt:
                break
            except EOFError:
                break
                
    except Exception as e:
        print(f"Error: {e}")
    finally:
        client.close()


if __name__ == "__main__":
    main()