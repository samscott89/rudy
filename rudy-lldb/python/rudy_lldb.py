#!/usr/bin/env python3
"""
rudy-lldb: Enhanced Rust debugging for LLDB

This module provides the `rd` command for LLDB with advanced expression evaluation
and pretty printing capabilities powered by rudy-db.
"""

from io import TextIOWrapper
import json

# ignore lldb since it doesn't have type hints
import lldb  # type: ignore
import os
import socket
import subprocess
import time
from typing import Optional


# Configuration
RUDY_HOST = os.environ.get("RUDY_HOST", "127.0.0.1")
RUDY_PORT = int(os.environ.get("RUDY_PORT", "9001"))
RUDY_DEBUG = os.environ.get("RUDY_DEBUG", "").lower() in ("1", "true", "on", "yes")


def debug_print(msg: str):
    """Print debug message if debug mode is enabled"""
    if RUDY_DEBUG:
        print(f"[DEBUG] {msg}")


class RudyConnection:
    """Manages connection to the RUDY-LLDB server"""

    file: TextIOWrapper

    def __init__(
        self,
        binary_path: str,
        host=RUDY_HOST,
        port=RUDY_PORT,
    ):
        """Connect to server and initialize session"""
        self.host = host
        self.port = port
        debug_print(f"Connecting to {self.host}:{self.port}")
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.connect((self.host, self.port))
        self.file = self.sock.makefile("rw")

        # Send init message
        init_msg = {"type": "Init", "binary_path": binary_path}
        debug_print(f"Sending init message: {init_msg}")
        self._send_message(init_msg)
        debug_print("Connected and init sent")
        self.request_id = 0

    def _send_message(self, msg):
        """Send a message to the server"""
        msg_json = json.dumps(msg) + "\n"
        self.file.write(msg_json)
        self.file.flush()

    def _read_message(self):
        """Read a message from the server"""

        line = self.file.readline().strip()
        if not line:
            raise ConnectionError("Server closed connection")
        return json.loads(line)

    def send_command(self, cmd: str, args: list, debugger) -> dict:
        """Send a command and handle event loop"""

        self.request_id += 1
        msg = {"type": "Command", "cmd": cmd, "args": args}

        self._send_message(msg)

        # Event loop - handle events until we get a final response
        while True:
            response = self._read_message()

            if response.get("type") == "Event":
                # Handle event request from server
                event_response = self._handle_event(response, debugger)
                self._send_message(event_response)
            elif response.get("type") in ["Complete", "Error"]:
                return response
            else:
                raise RuntimeError(f"Unknown response type: {response.get('type')}")

    def _handle_event(self, event_msg: dict, debugger) -> dict:
        """Handle an event request from the server"""
        event = event_msg.get("event")

        try:
            target = debugger.GetSelectedTarget()
            if not target:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No target selected",
                }

            process = target.GetProcess()
            if not process:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No process",
                }

            if event == "ReadMemory":
                return self._handle_read_memory(event_msg, process)
            elif event == "ReadRegister":
                return self._handle_read_register(event_msg, process)
            elif event == "ReadRegisterByIndex":
                return self._handle_read_register_by_index(event_msg, process)
            elif event == "GetFrameInfo":
                return self._handle_get_frame_info(process)
            elif event == "GetThreadInfo":
                return self._handle_get_thread_info(process)
            elif event == "GetBaseAddress":
                return self._handle_get_base_address(target)
            elif event == "EvaluateLLDBExpression":
                return self._handle_evaluate_expression(event_msg, target)
            else:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": f"Unknown event type: {event}",
                }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Event handling error: {e}",
            }

    def _handle_read_memory(self, event_msg: dict, process) -> dict:
        """Handle ReadMemory event"""
        address = event_msg.get("address", 0)
        size = event_msg.get("size", 0)

        try:
            # Read memory
            error = lldb.SBError()
            data = process.ReadMemory(address, size, error)

            if error.Success():
                return {
                    "type": "EventResponse",
                    "event": "MemoryData",
                    "data": list(data),  # Convert bytes to list of ints
                }
            else:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": f"Memory read failed: {error.GetCString()}",
                }
        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Memory read error: {e}",
            }

    def _handle_read_register(self, event_msg: dict, process) -> dict:
        """Handle ReadRegister event"""
        reg_name = event_msg.get("name", "")

        try:
            thread = process.GetSelectedThread()
            if not thread:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No selected thread",
                }

            frame = thread.GetSelectedFrame()
            if not frame:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No selected frame",
                }

            # Find register
            registers = frame.GetRegisters()
            for reg_set in registers:
                for reg in reg_set:
                    if reg.GetName() == reg_name:
                        # Convert register value to integer
                        reg_value = int(reg.GetValueAsUnsigned())

                        return {
                            "type": "EventResponse",
                            "event": "RegisterData",
                            "value": reg_value,
                        }

            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Register '{reg_name}' not found",
            }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Register read error: {e}",
            }

    def _handle_read_register_by_index(self, event_msg: dict, process) -> dict:
        """Handle ReadRegisterByIndex event"""
        reg_index = event_msg.get("index", 0)

        try:
            thread = process.GetSelectedThread()
            if not thread:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No selected thread",
                }

            frame = thread.GetSelectedFrame()
            if not frame:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No selected frame",
                }

            # Get all register sets and find the register by index
            registers = frame.GetRegisters()
            current_index = 0

            debug_print(f"Looking for register index {reg_index}")

            for reg_set in registers:
                if reg_set.GetNumChildren() >= reg_index:
                    reg = reg_set.GetChildAtIndex(reg_index)
                    if reg:
                        reg_value = int(reg.GetData().uint64s[0])
                        debug_print(
                            f"Found register {reg_index} ({reg.GetName()}): {reg_value:#x}"
                        )
                        return {
                            "type": "EventResponse",
                            "event": "RegisterData",
                            "value": reg_value,
                        }
                    else:
                        debug_print(f"Register at index {reg_index} not found in set")

            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Register index {reg_index} not found (only have {current_index} registers)",
            }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Register read by index error: {e}",
            }

    def _handle_get_frame_info(self, process) -> dict:
        """Handle GetFrameInfo event"""
        try:
            thread = process.GetSelectedThread()
            if not thread:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No selected thread",
                }

            frame = thread.GetSelectedFrame()
            if not frame:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No selected frame",
                }

            pc = frame.GetPC()
            sp = frame.GetSP()
            fp = frame.GetFP()

            return {
                "type": "EventResponse",
                "event": "FrameInfo",
                "pc": pc,
                "sp": sp,
                "fp": fp,
            }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Frame info error: {e}",
            }

    def _handle_get_thread_info(self, process) -> dict:
        """Handle GetThreadInfo event"""
        try:
            thread = process.GetSelectedThread()
            if not thread:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No selected thread",
                }

            return {
                "type": "EventResponse",
                "event": "ThreadInfo",
                "tid": thread.GetThreadID(),
                "name": thread.GetName() or None,
            }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Thread info error: {e}",
            }

    def _handle_get_base_address(self, target) -> dict:
        """Handle GetBaseAddress event"""
        try:
            if not target:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No target available",
                }

            # Get the main module (executable)
            module = target.GetModuleAtIndex(0)
            if not module:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No modules loaded",
                }

            # Get the base address where the module is loaded
            base_address = module.GetObjectFileHeaderAddress().GetLoadAddress(target)

            return {
                "type": "EventResponse",
                "event": "BaseAddress",
                "address": base_address,
            }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Base address error: {e}",
            }

    def _handle_evaluate_expression(self, event_msg: dict, target) -> dict:
        """Handle EvaluateLLDBExpression event"""
        expr = event_msg.get("expr", "")

        try:
            # Use LLDB's expression evaluator
            result = target.EvaluateExpression(expr)

            if result.IsValid():
                return {
                    "type": "EventResponse",
                    "event": "ExpressionResult",
                    "value": str(result.GetValue() or result.GetSummary() or ""),
                }
            else:
                error = result.GetError()
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": f"Expression evaluation failed: {error.GetCString() if error else 'Unknown error'}",
                }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Expression evaluation error: {e}",
            }

    def __del__(self):
        """Close the connection"""
        try:
            self.file.close()
        except Exception as _:
            pass
        if self.sock:
            try:
                self.sock.close()
            except Exception as _:
                pass
            self.sock = None


# Global connection instance
_connection: Optional[RudyConnection] = None


def _ensure_server_running() -> bool:
    """Ensure the Rudy server is running"""
    try:
        # Try to connect briefly to see if server is up
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.settimeout(1)
        result = sock.connect_ex((RUDY_HOST, RUDY_PORT))
        sock.close()
        return result == 0
    except Exception as e:
        print(f"Error checking server status: {e}")
        return False


def _start_server() -> bool:
    """Start the Rudy server if not running"""
    if _ensure_server_running():
        return True

    try:
        # Try to start the server (TODO: should look for binary in PATH)
        print(f"Starting Rudy server on {RUDY_HOST}:{RUDY_PORT}...")
        subprocess.Popen(
            [
                "cargo",
                "run",
                "-p",
                "rudy-lldb",
                "--bin",
                "rudy-lldb-server",
                "--",
                "--port",
                str(RUDY_PORT),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        # Wait a bit for server to start
        for _ in range(10):
            time.sleep(0.5)
            if _ensure_server_running():
                print("Rudy server started successfully")
                return True

        print("Failed to start Rudy server")
        return False

    except Exception as e:
        print(f"Error starting server: {e}")
        return False


def _get_connection(debugger) -> Optional[RudyConnection]:
    """Get or create connection to Rudy server"""
    global _connection

    # Get current executable path
    target = debugger.GetSelectedTarget()
    if not target:
        print("No target selected")
        return None

    executable = target.GetExecutable()
    if not executable:
        print("No executable in target")
        return None

    binary_path = executable.GetDirectory() + "/" + executable.GetFilename()
    debug_print(f"Binary path: {binary_path}")

    # Check if we have a valid connection
    if _connection:
        debug_print("Reusing existing connection")
        return _connection

    debug_print("Need new connection")

    # Ensure server is running
    if not _ensure_server_running():
        debug_print("Server not running, starting...")
        if not _start_server():
            return None
    else:
        debug_print("Server already running")

    # Create new connection
    debug_print("Creating new RudyConnection...")
    debug_print(f"Calling connect with: {binary_path}")
    try:
        _connection = RudyConnection(binary_path)
        debug_print("Connection successful")
        return _connection
    except Exception as e:
        debug_print(f"Connection failed: {e}")
        _connection = None
        return None


def rudy_command(debugger, command, result, internal_dict):
    """Main Rudy command handler"""
    args = command.strip().split()
    if not args:
        print("Usage: rd <subcommand> [args...]")
        print("Available commands:")
        print("  eval <expression>  - Evaluate a Rust expression")
        print("  print <expression> - Pretty print a Rust expression")
        print("  status             - Show Rudy server status")
        return

    shortcodes = {
        "e": "eval",
        "m": "methods",
        "p": "print",
        "s": "status",
    }

    subcommand = args[0]
    cmd_args = args[1:]

    # Map shortcodes to full commands
    if subcommand in shortcodes:
        subcommand = shortcodes[subcommand]

    if subcommand == "status":
        if _ensure_server_running():
            print(f"Rudy server is running on {RUDY_HOST}:{RUDY_PORT}")
        else:
            print("Rudy server is not running")
        return

    # Get connection
    conn = _get_connection(debugger)
    if not conn:
        print("Failed to connect to Rudy server")
        return

    # Send command
    try:
        response = conn.send_command(subcommand, cmd_args, debugger)

        # Handle response
        if response.get("type") == "Complete":
            result_data = response.get("result", {})
            if isinstance(result_data, dict):
                for key, value in result_data.items():
                    print(f"{key}: {value}")
            else:
                print(result_data)
        elif response.get("type") == "Error":
            print(f"Error: {response.get('error', 'Unknown error')}")
        else:
            print(f"Unexpected response: {response}")
    except Exception as e:
        print(f"Command failed: {e}")


def __lldb_init_module(debugger, internal_dict):
    """Initialize the rudy-lldb module"""
    debugger.HandleCommand("command script add -f rudy_lldb.rudy_command rd")

    print("rudy-lldb extension loaded!")
    print("Available commands:")
    print("  rd eval <expression>  - Evaluate Rust expressions")
    print("  rd print <expression> - Pretty print Rust values")
    print("  rd status             - Check server status")
    print("")
    print(f"Server: {RUDY_HOST}:{RUDY_PORT}")

    # Check server status
    if _ensure_server_running():
        print("✓ Rudy server is already running")
    else:
        print("⚠ Rudy server not running (will auto-start when needed)")
