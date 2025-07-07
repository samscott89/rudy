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
            elif event == "ExecuteMethod":
                return self._handle_execute_method(event_msg, target)
            elif event == "ExecuteFunction":
                return self._handle_execute_function(event_msg, target)
            elif event == "GetVariableType":
                return self._handle_get_variable_type(event_msg, target)
            elif event == "AllocateMemory":
                return self._handle_allocate_memory(event_msg, target)
            elif event == "WriteMemory":
                return self._handle_write_memory(event_msg, target)
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

    def _handle_execute_method(self, event_msg: dict, target) -> dict:
        """Handle ExecuteMethod event with support for complex return types"""
        method_address = event_msg.get("method_address", 0)
        base_address = event_msg.get("base_address", 0)
        args = event_msg.get("args", [])
        return_type_size = event_msg.get("return_type_size")

        try:
            debug_print(f"Executing method at {method_address:#x}")
            debug_print(f"Base address: {base_address:#x}")
            debug_print(f"Return type size: {return_type_size}")

            # Build argument types and values for the call
            arg_types = ["void*"]  # Always start with &self
            arg_values = [f"(void*){base_address:#x}"]

            # Add other arguments
            for arg in args:
                arg_type = arg.get("arg_type", "Integer")
                arg_value = arg.get("value", "0")

                if arg_type == "Pointer":
                    arg_types.append("void*")
                    arg_values.append(f"(void*){arg_value}")
                elif arg_type == "Integer":
                    arg_types.append("unsigned long")
                    arg_values.append(arg_value)
                elif arg_type == "Bool":
                    arg_types.append("int")
                    arg_values.append(
                        "1" if arg_value.lower() in ("true", "1") else "0"
                    )
                elif arg_type == "Float":
                    arg_types.append("double")
                    arg_values.append(arg_value)
                else:
                    arg_types.append("unsigned long")
                    arg_values.append(arg_value)

            if return_type_size is None:
                # Simple return type - direct call returning value in register
                func_signature = (
                    f"((unsigned long (*)({', '.join(arg_types)})){method_address:#x})"
                )
                call_expr = f"{func_signature}({', '.join(arg_values)})"

                debug_print(f"Simple call: {call_expr}")
                result = target.EvaluateExpression(call_expr)

                if result.IsValid() and not result.GetError().Fail():
                    value = result.GetValueAsUnsigned()
                    return {
                        "type": "EventResponse",
                        "event": "MethodResult",
                        "result": {
                            "SimpleValue": {
                                "value": value,
                                "return_type": "usize",  # Default for simple types
                            }
                        },
                    }
                else:
                    error = result.GetError()
                    return {
                        "type": "EventResponse",
                        "event": "Error",
                        "message": f"Simple method call failed: {error.GetCString() if error else 'Unknown error'}",
                    }
            else:
                # Complex return type - use byte array struct approach
                size = return_type_size

                # Create a struct to hold the return value
                struct_def = (
                    f"struct ReturnBuffer_{size} {{ unsigned char bytes[{size}]; }}"
                )
                func_signature = f"((struct ReturnBuffer_{size} (*)({', '.join(arg_types)})){method_address:#x})"
                call_expr = f"{struct_def}; auto result = {func_signature}({', '.join(arg_values)}); &result"

                debug_print(f"Complex call: {call_expr}")
                result = target.EvaluateExpression(call_expr)

                if result.IsValid() and not result.GetError().Fail():
                    # Get the address of the result struct
                    # Since we used &result, this should be a pointer to the struct
                    result_addr = result.GetValueAsUnsigned()
                    if result_addr == 0:
                        # Fallback to load address method
                        result_addr = result.GetLoadAddress()
                        if result_addr == lldb.LLDB_INVALID_ADDRESS:
                            result_addr = result.GetAddress().GetLoadAddress(target)

                    debug_print(f"Complex result at address: {result_addr:#x}")
                    debug_print(f"Result value type: {result.GetTypeName()}")

                    if result_addr != lldb.LLDB_INVALID_ADDRESS:
                        return {
                            "type": "EventResponse",
                            "event": "MethodResult",
                            "result": {
                                "ComplexPointer": {
                                    "address": result_addr,
                                    "size": size,
                                    "return_type": "complex",
                                }
                            },
                        }
                    else:
                        return {
                            "type": "EventResponse",
                            "event": "Error",
                            "message": "Could not get address of complex return value",
                        }
                else:
                    error = result.GetError()
                    return {
                        "type": "EventResponse",
                        "event": "Error",
                        "message": f"Complex method call failed: {error.GetCString() if error else 'Unknown error'}",
                    }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Method execution error: {e}",
            }

    def _handle_execute_function(self, event_msg: dict, target) -> dict:
        """Handle ExecuteFunction event with support for complex return types"""
        function_address = event_msg.get("function_address", 0)
        args = event_msg.get("args", [])
        return_type_size = event_msg.get("return_type_size")

        try:
            debug_print(f"Executing function at {function_address:#x}")
            debug_print(f"Return type size: {return_type_size}")

            # Build argument types and values for the call
            arg_types = []
            arg_values = []

            # Add function arguments
            for arg in args:
                arg_type = arg.get("arg_type", "Integer")
                arg_value = arg.get("value", "0")

                if arg_type == "Pointer":
                    arg_types.append("void*")
                    arg_values.append(f"(void*)0x{arg_value}")
                elif arg_type == "Integer":
                    arg_types.append("unsigned long")
                    arg_values.append(arg_value)
                elif arg_type == "Bool":
                    arg_types.append("int")
                    arg_values.append(
                        "1" if arg_value.lower() in ("true", "1") else "0"
                    )
                elif arg_type == "Float":
                    arg_types.append("double")
                    arg_values.append(arg_value)
                else:
                    arg_types.append("unsigned long")
                    arg_values.append(arg_value)

            if return_type_size is None:
                # Simple return type - direct call returning value in register
                func_signature = f"((unsigned long (*)({', '.join(arg_types) if arg_types else 'void'})){function_address:#x})"
                call_expr = f"{func_signature}({', '.join(arg_values)})"

                debug_print(f"Simple function call: {call_expr}")
                result = target.EvaluateExpression(call_expr)

                if result.IsValid() and not result.GetError().Fail():
                    value = result.GetValueAsUnsigned()
                    return {
                        "type": "EventResponse",
                        "event": "FunctionResult",
                        "result": {
                            "SimpleValue": {
                                "value": value,
                                "return_type": "usize",  # Default for simple types
                            }
                        },
                    }
                else:
                    error = result.GetError()
                    return {
                        "type": "EventResponse",
                        "event": "Error",
                        "message": f"Simple function call failed: {error.GetCString() if error else 'Unknown error'}",
                    }
            else:
                # Complex return type - use byte array struct approach
                size = return_type_size

                # Create a struct to hold the return value
                struct_def = (
                    f"struct ReturnBuffer_{size} {{ unsigned char bytes[{size}]; }}"
                )
                func_signature = f"((struct ReturnBuffer_{size} (*)({', '.join(arg_types) if arg_types else 'void'})){function_address:#x})"
                call_expr = f"{struct_def}; auto result = {func_signature}({', '.join(arg_values)}); &result"

                debug_print(f"Complex function call: {call_expr}")
                result = target.EvaluateExpression(call_expr)

                if result.IsValid() and not result.GetError().Fail():
                    # Get the address of the result struct
                    # Since we used &result, this should be a pointer to the struct
                    result_addr = result.GetValueAsUnsigned()
                    if result_addr == 0:
                        # Fallback to load address method
                        result_addr = result.GetLoadAddress()
                        if result_addr == lldb.LLDB_INVALID_ADDRESS:
                            result_addr = result.GetAddress().GetLoadAddress(target)

                    debug_print(f"Complex function result at address: {result_addr:#x}")
                    debug_print(f"Result value type: {result.GetTypeName()}")

                    if result_addr != lldb.LLDB_INVALID_ADDRESS:
                        return {
                            "type": "EventResponse",
                            "event": "FunctionResult",
                            "result": {
                                "ComplexPointer": {
                                    "address": result_addr,
                                    "size": size,
                                    "return_type": "complex",
                                }
                            },
                        }
                    else:
                        return {
                            "type": "EventResponse",
                            "event": "Error",
                            "message": "Could not get address of complex function return value",
                        }
                else:
                    error = result.GetError()
                    return {
                        "type": "EventResponse",
                        "event": "Error",
                        "message": f"Complex function call failed: {error.GetCString() if error else 'Unknown error'}",
                    }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Function execution error: {e}",
            }

    def _handle_get_variable_type(self, event_msg: dict, target) -> dict:
        """Handle GetVariableType event"""
        var_name = event_msg.get("name", "")

        try:
            debug_print(f"Getting type for variable: {var_name}")

            # Get the current frame
            process = target.GetProcess()
            if not process:
                return {
                    "type": "EventResponse",
                    "event": "VariableTypeResult",
                    "type_name": None,
                }

            thread = process.GetSelectedThread()
            if not thread:
                return {
                    "type": "EventResponse",
                    "event": "VariableTypeResult",
                    "type_name": None,
                }

            frame = thread.GetSelectedFrame()
            if not frame:
                return {
                    "type": "EventResponse",
                    "event": "VariableTypeResult",
                    "type_name": None,
                }

            # Try to find the variable in the current frame
            var = frame.FindVariable(var_name)
            if var.IsValid():
                type_name = var.GetTypeName()
                return {
                    "type": "EventResponse",
                    "event": "VariableTypeResult",
                    "type_name": type_name,
                }
            else:
                # Variable not found
                return {
                    "type": "EventResponse",
                    "event": "VariableTypeResult",
                    "type_name": None,
                }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Variable type query error: {e}",
            }

    def _handle_allocate_memory(self, event_msg: dict, target) -> dict:
        """Handle AllocateMemory event"""
        size = event_msg.get("size", 0)

        try:
            debug_print(f"Allocating {size} bytes in target process")

            # Use LLDB's expression evaluator to allocate memory
            # malloc is typically available in most processes
            alloc_expr = f"(void*)malloc({size})"
            result = target.EvaluateExpression(alloc_expr)

            if result.IsValid() and not result.GetError().Fail():
                address = result.GetValueAsUnsigned()
                debug_print(f"Allocated memory at address: {address:#x}")
                return {
                    "type": "EventResponse",
                    "event": "MemoryAllocated",
                    "address": address,
                }
            else:
                error = result.GetError()
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": f"Memory allocation failed: {error.GetCString() if error else 'Unknown error'}",
                }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Memory allocation error: {e}",
            }

    def _handle_write_memory(self, event_msg: dict, target) -> dict:
        """Handle WriteMemory event"""
        address = event_msg.get("address", 0)
        data = event_msg.get("data", [])

        try:
            debug_print(f"Writing {len(data)} bytes to address {address:#x}")

            # Get the process
            process = target.GetProcess()
            if not process:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": "No process available for memory write",
                }

            # Convert list of ints to bytes
            data_bytes = bytes(data)

            # Write memory
            error = lldb.SBError()
            bytes_written = process.WriteMemory(address, data_bytes, error)

            if error.Success() and bytes_written == len(data_bytes):
                debug_print(f"Successfully wrote {bytes_written} bytes")
                return {
                    "type": "EventResponse",
                    "event": "MemoryWritten",
                }
            else:
                return {
                    "type": "EventResponse",
                    "event": "Error",
                    "message": f"Memory write failed: {error.GetCString() if error else 'Unknown error'}",
                }

        except Exception as e:
            return {
                "type": "EventResponse",
                "event": "Error",
                "message": f"Memory write error: {e}",
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
        print("  eval <expression>     - Evaluate a Rust expression")
        print("  methods <expression>  - List all methods for a type or expression")
        print("  functions [pattern]   - List all functions or search by pattern")
        print("  print <expression>    - Pretty print a Rust expression")
        print("  status                - Show Rudy server status")
        return

    shortcodes = {
        "e": "eval",
        "f": "functions",
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

        debug_print(f"Received response: {response}")

        # Handle response
        if response.get("type") == "Complete":
            result_data = response.get("result", {})

            # Special formatting for methods command
            if subcommand == "methods" and isinstance(result_data, dict):
                type_name = result_data.get("type_name", "Unknown")
                methods = result_data.get("methods", [])

                # Separate regular and synthetic methods
                regular_methods = []
                synthetic_methods = []

                for method in methods:
                    if isinstance(method, dict):
                        if method.get("is_synthetic", False):
                            synthetic_methods.append(method)
                        else:
                            regular_methods.append(method)

                print(f"Methods for {type_name}:")

                # Print regular methods
                if regular_methods:
                    for method in regular_methods:
                        sig = method.get("signature", "")
                        callable_str = (
                            " (callable)"
                            if method.get("callable", False)
                            else " (not callable)"
                        )
                        print(f"  - {sig}{callable_str}")
                else:
                    print("  (no methods found)")

                # Print synthetic methods if any
                if synthetic_methods:
                    print("\nSynthetic methods (debug helpers):")
                    for method in synthetic_methods:
                        sig = method.get("signature", "")
                        print(f"  - {sig}")

            # Special formatting for functions command
            elif subcommand == "functions" and isinstance(result_data, list):
                if result_data:
                    print(f"Found {len(result_data)} function(s):")
                    for func in result_data:
                        if isinstance(func, dict):
                            name = func.get("name", "")
                            signature = func.get("signature", "")
                            callable_str = (
                                " (callable)"
                                if func.get("callable", False)
                                else " (not callable)"
                            )
                            module_path = func.get("module_path", [])
                            if module_path:
                                full_path = "::".join(module_path + [name])
                                print(f"  {full_path}: {signature}{callable_str}")
                            else:
                                print(f"  {name}: {signature}{callable_str}")
                else:
                    print("No functions found")
            elif isinstance(result_data, dict):
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
    print("  rd eval <expression>    - Evaluate Rust expressions")
    print("  rd functions [pattern]  - List all functions or search by pattern")
    print("  rd print <expression>   - Pretty print Rust values")
    print("  rd status               - Check server status")
    print("")
    print(f"Server: {RUDY_HOST}:{RUDY_PORT}")

    # Check server status
    if _ensure_server_running():
        print("✓ Rudy server is already running")
    else:
        print("⚠ Rudy server not running (will auto-start when needed)")
