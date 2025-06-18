#!/usr/bin/env python3
"""
LLDB integration for rust-debuginfo

Usage:
1. Start the rust-debuginfo server:
   cargo run --example lldb_server -- /path/to/binary

2. In LLDB:
   (lldb) command script import /path/to/rust_debuginfo_lldb.py
   (lldb) rdi print 0x12345678
   (lldb) rdi types MyStruct
   (lldb) rdi find Vec
"""

import lldb
import json
import socket
import os

# Configuration
RDI_HOST = os.environ.get('RDI_HOST', '127.0.0.1')
RDI_PORT = int(os.environ.get('RDI_PORT', '9001'))

class RustDebugInfoClient:
    def __init__(self, host=RDI_HOST, port=RDI_PORT):
        self.host = host
        self.port = port
        self.request_id = 0

    def _send_request(self, method, params):
        """Send JSON-RPC request to rust-debuginfo server"""
        self.request_id += 1
        request = {
            'method': method,
            'params': params,
            'id': self.request_id
        }
        
        try:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.connect((self.host, self.port))
            
            # Send request
            request_str = json.dumps(request) + '\n'
            sock.send(request_str.encode())
            
            # Read response
            response_data = b''
            while b'\n' not in response_data:
                chunk = sock.recv(4096)
                if not chunk:
                    break
                response_data += chunk
            
            sock.close()
            
            response = json.loads(response_data.decode().strip())
            if response.get('error'):
                print(f"Error: {response['error']}")
                return None
            return response.get('result')
            
        except Exception as e:
            print(f"Failed to connect to rust-debuginfo server: {e}")
            print(f"Make sure server is running on {self.host}:{self.port}")
            return None

    def pretty_print(self, address, max_depth=3):
        """Pretty print value at address"""
        params = {
            'address': f'0x{address:x}',
            'max_depth': max_depth
        }
        return self._send_request('pretty_print', params)

    def type_layout(self, type_name):
        """Get memory layout of a type"""
        params = {'type_name': type_name}
        return self._send_request('type_layout', params)

    def find_type(self, pattern):
        """Find types matching pattern"""
        params = {'pattern': pattern}
        return self._send_request('find_type', params)

    def load_binary(self, binary_path):
        """Load a new binary into the server"""
        params = {'binary_path': binary_path}
        return self._send_request('load_binary', params)


# Global client instance
rdi_client = RustDebugInfoClient()


def rdi_print(debugger, command, result, internal_dict):
    """Pretty print value at address using rust-debuginfo"""
    args = command.split()
    if not args:
        print("Usage: rdi print <address>")
        return
    
    # Parse address
    addr_str = args[1]
    try:
        if addr_str.startswith('0x'):
            address = int(addr_str, 16)
        else:
            address = int(addr_str)
    except ValueError:
        print(f"Invalid address: {addr_str}")
        return
    
    # Get current target's executable path
    target = debugger.GetSelectedTarget()
    if target:
        executable = target.GetExecutable()
        if executable:
            binary_path = executable.GetDirectory() + "/" + executable.GetFilename()
            # First ensure server knows about this binary
            rdi_client.load_binary(binary_path)
    
    # Get pretty printed value
    response = rdi_client.pretty_print(address)
    if response:
        print(f"Type: {response.get('type', 'Unknown')}")
        print(f"Source: {response.get('source', 'Unknown')}")
        print("Value:")
        _print_value(response.get('value', {}), indent=2)


def rdi_types(debugger, command, result, internal_dict):
    """Show type information"""
    args = command.split()
    if not args:
        print("Usage: rdi types <type_name>")
        return
    
    type_name = args[0]
    response = rdi_client.type_layout(type_name)
    if response:
        print(f"Type: {response['name']}")
        print(f"Size: {response['size']} bytes")
        print(f"Alignment: {response['align']} bytes")
        print("Fields:")
        for field in response.get('fields', []):
            print(f"  [{field['offset']:04x}] {field['name']}: {field['type']} ({field['size']} bytes)")


def rdi_find(debugger, command, result, internal_dict):
    """Find types matching pattern"""
    args = command.split()
    if not args:
        print("Usage: rdi find <pattern>")
        return
    
    pattern = args[0]
    response = rdi_client.find_type(pattern)
    if response:
        types = response.get('types', [])
        if types:
            print(f"Found {len(types)} types matching '{pattern}':")
            for t in types:
                print(f"  {t['name']} (module: {t['module']})")
        else:
            print(f"No types found matching '{pattern}'")


def rdi_status(debugger, command, result, internal_dict):
    """Check connection to rust-debuginfo server"""
    # Try a simple request
    response = rdi_client.find_type("*")
    if response is not None:
        print(f"Connected to rust-debuginfo server at {rdi_client.host}:{rdi_client.port}")
    else:
        print(f"Not connected to rust-debuginfo server")
        print(f"Start server with: cargo run --example lldb_server -- /path/to/binary")


def _print_value(value, indent=0):
    """Pretty print a value with indentation"""
    prefix = ' ' * indent
    if isinstance(value, dict):
        for k, v in value.items():
            if isinstance(v, (dict, list)):
                print(f"{prefix}{k}:")
                _print_value(v, indent + 2)
            else:
                print(f"{prefix}{k}: {v}")
    elif isinstance(value, list):
        for i, item in enumerate(value):
            print(f"{prefix}[{i}]:")
            _print_value(item, indent + 2)
    else:
        print(f"{prefix}{value}")


def __lldb_init_module(debugger, internal_dict):
    """Initialize the rust-debuginfo LLDB commands"""
    debugger.HandleCommand('command script add -f rust_debuginfo_lldb.rdi_print rdi_print')
    debugger.HandleCommand('command script add -f rust_debuginfo_lldb.rdi_types rdi_types')
    debugger.HandleCommand('command script add -f rust_debuginfo_lldb.rdi_find rdi_find')
    debugger.HandleCommand('command script add -f rust_debuginfo_lldb.rdi_status rdi_status')
    
    # Shorter aliases
    debugger.HandleCommand('command alias rdi rdi_print')
    
    print("rust-debuginfo LLDB commands loaded. Available commands:")
    print("  rdi print <address>  - Pretty print value at address")
    print("  rdi types <type>     - Show type layout")
    print("  rdi find <pattern>   - Find types matching pattern")
    print("  rdi status           - Check server connection")
    print("")
    print(f"Connecting to server at {RDI_HOST}:{RDI_PORT}...")
    
    # Check connection
    response = rdi_client.find_type("*")
    if response is not None:
        print("Connected successfully!")
    else:
        print("Not connected. Start server with:")
        print("  cargo run --example lldb_server -- /path/to/binary")