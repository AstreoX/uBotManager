//! Small graph helpers over adjacency lists (`edges[node]` = nodes that `node` requires).

use std::collections::{HashMap, VecDeque};

/// Every dependency cycle, one per strongly connected component (or self-requirement), as the
/// shortest path from the component's lowest node back to itself: `[a, b, c, a]`.
/// Sorted by that lowest node.
pub fn find_cycles(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let components = strongly_connected_components(edges);
    let mut cycles = Vec::new();
    for component in &components {
        let start = *component.iter().min().expect("components are not empty");
        let is_cycle = component.len() > 1 || edges[start].contains(&start);
        if is_cycle {
            cycles.push(shortest_cycle(edges, start, component));
        }
    }
    cycles.sort_by_key(|cycle| cycle[0]);
    cycles
}

fn shortest_cycle(edges: &[Vec<usize>], start: usize, component: &[usize]) -> Vec<usize> {
    let mut parents: HashMap<usize, usize> = HashMap::new();
    let mut queue = VecDeque::from([start]);
    while let Some(node) = queue.pop_front() {
        for &next in &edges[node] {
            if !component.contains(&next) {
                continue;
            }
            if next == start {
                let mut path = vec![start];
                let mut cursor = node;
                while cursor != start {
                    path.insert(1, cursor);
                    cursor = parents[&cursor];
                }
                path.push(start);
                return path;
            }
            if next != start && !parents.contains_key(&next) {
                parents.insert(next, node);
                queue.push_back(next);
            }
        }
    }
    vec![start, start]
}

/// Tarjan's algorithm (iterative, so deep graphs cannot overflow the stack).
pub fn strongly_connected_components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let count = edges.len();
    let mut index_of = vec![usize::MAX; count];
    let mut low = vec![0usize; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut components = Vec::new();
    let mut next_index = 0;

    for root in 0..count {
        if index_of[root] != usize::MAX {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index_of[root] = next_index;
        low[root] = next_index;
        next_index += 1;
        stack.push(root);
        on_stack[root] = true;

        while let Some(&mut (node, ref mut edge_position)) = work.last_mut() {
            if *edge_position < edges[node].len() {
                let next = edges[node][*edge_position];
                *edge_position += 1;
                if index_of[next] == usize::MAX {
                    index_of[next] = next_index;
                    low[next] = next_index;
                    next_index += 1;
                    stack.push(next);
                    on_stack[next] = true;
                    work.push((next, 0));
                } else if on_stack[next] {
                    low[node] = low[node].min(index_of[next]);
                }
                continue;
            }

            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index_of[node] {
                let mut component = Vec::new();
                loop {
                    let member = stack.pop().expect("node is on the stack");
                    on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                components.push(component);
            }
        }
    }
    components
}

/// Post-order over `roots` and everything they reach: dependencies come before their dependents.
/// Each node appears once; cycles are cut where they close.
pub fn dependency_order(edges: &[Vec<usize>], roots: &[usize]) -> Vec<usize> {
    let mut visited = vec![false; edges.len()];
    let mut order = Vec::new();
    for &root in roots {
        if visited[root] {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        visited[root] = true;
        while let Some(&mut (node, ref mut edge_position)) = work.last_mut() {
            if *edge_position < edges[node].len() {
                let next = edges[node][*edge_position];
                *edge_position += 1;
                if !visited[next] {
                    visited[next] = true;
                    work.push((next, 0));
                }
                continue;
            }
            work.pop();
            order.push(node);
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_are_found_once_per_component() {
        // A -> B -> C -> A, D -> A, E -> E, F -> G (G -> F is the only edge back, optional edges are
        // not passed in).
        let edges = vec![vec![1], vec![2], vec![0], vec![0], vec![4], vec![6], vec![]];
        assert_eq!(find_cycles(&edges), vec![vec![0, 1, 2, 0], vec![4, 4]]);
        assert!(find_cycles(&[vec![1], vec![]]).is_empty());
        // Shortest path back to the start.
        let edges = vec![vec![1, 2], vec![2], vec![0]];
        assert_eq!(find_cycles(&edges), vec![vec![0, 2, 0]]);
    }

    #[test]
    fn dependency_order_puts_dependencies_first() {
        // Diamond: 2 (Top) -> 3 (Left), 1 (Right); Left -> 0 (Base); Right -> 0.
        let edges = vec![vec![], vec![0], vec![3, 1], vec![0]];
        assert_eq!(dependency_order(&edges, &[2]), vec![0, 3, 1, 2]);
        assert_eq!(dependency_order(&edges, &[3, 2]), vec![0, 3, 1, 2]);
        let cyclic = vec![vec![1], vec![0]];
        assert_eq!(dependency_order(&cyclic, &[0]), vec![1, 0]);
    }
}
