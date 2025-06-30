use std::fmt;

use crate::file::DebugFile;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FunctionAddressInfo {
    /// The start address of the function
    ///
    /// These are absolute addresses in the binary, not relative to any debug file.
    pub start: u64,
    pub end: u64,
    pub relative_start: u64,
    pub file: DebugFile,
    pub name: crate::dwarf::SymbolName,
}

impl fmt::Debug for FunctionAddressInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        salsa::with_attached_database(|db| {
            let file_path = self.file.file(db).path(db);
            f.debug_struct("FunctionAddressInfo")
                .field("start", &self.start)
                .field("end", &self.end)
                .field("relative_start", &self.relative_start)
                .field("file", &file_path)
                .field("name", &self.name)
                .finish()
        })
        .unwrap_or_else(|| {
            f.debug_struct("FunctionAddressInfo")
                .field("start", &self.start)
                .field("end", &self.end)
                .field("relative_start", &self.relative_start)
                .finish()
        })
    }
}

impl FunctionAddressInfo {
    fn contains_address(&self, address: u64) -> bool {
        address >= self.start && address < self.end
    }

    fn overlaps(&self, start: u64, end: u64) -> bool {
        // Check if the current function's range overlaps with the query range
        self.start < end && start < self.end
    }
}

// Define the Node for the Interval Tree
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Node {
    info: FunctionAddressInfo,
    max_end_in_subtree: u64, // Maximum endpoint in the subtree rooted at this node
    left: Option<Box<Node>>,
    right: Option<Box<Node>>,
}

impl Node {
    fn new(info: FunctionAddressInfo) -> Self {
        Node {
            max_end_in_subtree: info.end,
            info,
            left: None,
            right: None,
        }
    }

    fn update_max_end(&mut self) {
        self.max_end_in_subtree = self.info.end;
        if let Some(ref left_child) = self.left {
            self.max_end_in_subtree =
                std::cmp::max(self.max_end_in_subtree, left_child.max_end_in_subtree);
        }
        if let Some(ref right_child) = self.right {
            self.max_end_in_subtree =
                std::cmp::max(self.max_end_in_subtree, right_child.max_end_in_subtree);
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct AddressTree {
    root: Option<Box<Node>>,
}

impl AddressTree {
    pub fn new(mut info: Vec<FunctionAddressInfo>) -> Self {
        /// Recursive helper function to build the tree from a sorted slice of addresses.
        fn build_recursive(sorted_ranges: &[FunctionAddressInfo]) -> Option<Box<Node>> {
            if sorted_ranges.is_empty() {
                return None;
            }

            let mid_idx = sorted_ranges.len() / 2;
            let median_range = sorted_ranges[mid_idx].clone(); // This range becomes the root

            let mut node = Box::new(Node::new(median_range));

            // Recursively build the left subtree from the part of the slice to the left of the median
            node.left = build_recursive(&sorted_ranges[0..mid_idx]);

            // Recursively build the right subtree from the part of the slice to the right of the median
            node.right = build_recursive(&sorted_ranges[mid_idx + 1..]);

            // Update the max_end_in_subtree for the current node after its children are built
            node.update_max_end();

            Some(node)
        }

        if info.is_empty() {
            return AddressTree::default();
        }

        info.sort_unstable_by_key(|i| i.start); // Sort intervals by start point

        let root = build_recursive(&info);
        AddressTree { root }
    }

    /// Finds all functions in the tree that contain the given address.
    pub fn query_address(&self, address: u64) -> Vec<&FunctionAddressInfo> {
        let mut result = Vec::new();
        if let Some(ref root_node) = self.root {
            Self::query_address_recursive(root_node, address, &mut result);
        }

        if result.is_empty() {
            tracing::debug!("No function found for address {address} in {self:#?}");
        }

        result
    }

    fn query_address_recursive<'a>(
        node: &'a Node,
        address: u64,
        result: &mut Vec<&'a FunctionAddressInfo>,
    ) {
        // 1. Check if the address is contained in the current node's range
        if node.info.contains_address(address) {
            result.push(&node.info);
        }

        // 2. Check left child:
        //    The point must be less than or equal to the max_end in the left subtree
        //    AND (for this simple BST-like insertion based on start point) the point
        //    could potentially be in an interval starting to its left.
        if let Some(ref left_child) = node.left {
            // Pruning: if the query point is beyond the maximum reach of the left subtree, don't go there.
            if address <= left_child.max_end_in_subtree {
                // Crucial pruning step
                Self::query_address_recursive(left_child, address, result);
            }
        }

        // 3. Check right child:
        //    The point must be greater than or equal to the start of the current node's interval
        //    (because intervals in the right subtree start at or after the current node's interval start).
        //    AND the point must be less than or equal to the max_end in the right subtree.
        //
        //    For point queries, the `max_end_in_subtree` check is still useful.
        //    If the point is less than the start of an interval, it cannot be contained.
        if let Some(ref right_child) = node.right {
            // Pruning: if the query point is beyond the maximum reach of the right subtree, don't go there.
            if address <= right_child.max_end_in_subtree && address >= node.info.start {
                // The second condition (point >= node.info.start) is because our insertion
                // puts smaller start times to the left. If the point is smaller than the
                // current node's interval's start, it won't be in the right subtree intervals
                // which all have starts >= current node's interval start.
                // However, for point query, the primary condition is `point <= right_child.max_end_in_subtree`.
                // Let's refine the condition for going right.
                // We go right if the point *could* be in an interval there.
                // An interval [s,e] in the right subtree has s >= node.info.start.
                // So if point < node.info.start, it can't be in any interval in the right subtree.
                // Combined with max_end_in_subtree:
                if address >= node.info.start || address <= right_child.max_end_in_subtree {
                    // The check `point >= node.info.start` is more about interval overlap queries.
                    // For a point query, if point > node.info.start, it *might* be in the right.
                    // If point < node.info.start, it *might* still be in the right if an interval there
                    // starts small but extends far.
                    // The crucial pruning is max_end_in_subtree.
                    // If point > right_child.max_end_in_subtree, no interval in the right child can contain it.
                    //
                    // Let's simplify the traversal logic for point query:
                    // If the node's interval's start point is to the left of or at the query point,
                    // and the max_end_in_subtree of the right child extends to or past the query point,
                    // then the query point *could* be in an interval in the right subtree.
                    if address <= right_child.max_end_in_subtree {
                        // Check if we even need to go right
                        Self::query_address_recursive(right_child, address, result);
                    }
                }
            }
        }
    }

    #[allow(dead_code)]
    /// Finds all functions in the tree that overlap with the given address range.
    pub fn query_address_range(
        &self,
        query_start: u64,
        query_end: u64,
    ) -> Vec<&FunctionAddressInfo> {
        let mut result = Vec::new();
        if let Some(ref root_node) = self.root {
            Self::query_interval_recursive(root_node, query_start, query_end, &mut result);
        }
        result
    }

    fn query_interval_recursive<'a>(
        node: &'a Node,
        query_start: u64,
        query_end: u64,
        result: &mut Vec<&'a FunctionAddressInfo>,
    ) {
        // 1. Check if current node overlaps with the query range
        if node.info.overlaps(query_start, query_end) {
            result.push(&node.info);
        }

        // 2. If left child exists and its max_end_in_subtree overlaps the query_start
        if let Some(ref left_child) = node.left {
            if left_child.max_end_in_subtree >= query_start {
                Self::query_interval_recursive(left_child, query_start, query_end, result);
            }
        }

        // 3. If right child exists and the current node's interval starts before query_end
        //    and the right child's max_end_in_subtree is relevant
        if let Some(ref right_child) = node.right {
            // The query interval must potentially overlap with intervals in the right subtree.
            // Intervals in the right subtree start at or after node.info.start.
            // The query must extend to or past node.info.start.
            // And the query must not end before any interval in the right subtree could start.
            if node.info.start <= query_end && // Current node's interval allows going right
               right_child.max_end_in_subtree >= query_start
            // Right subtree might contain an overlap
            {
                Self::query_interval_recursive(right_child, query_start, query_end, result);
            }
        }
    }
}
