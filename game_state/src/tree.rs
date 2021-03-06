use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};

type NodeMut<T> = RefCell<Node<T>>;
pub type RcNode<T> = Rc<NodeMut<T>>;
pub type WeakNode<T> = Weak<NodeMut<T>>;

static GLOBAL_NODE_ID: AtomicUsize = AtomicUsize::new(0);

pub trait NodeVisitor<T> {
    fn visit<F: FnMut(&T) -> ()>(&mut self, func: F);
    fn has_next(&self) -> bool;
}

#[derive(Default)]
pub struct Node<T> {
    pub id: usize,
    parent: Option<WeakNode<T>>,
    children: Vec<RcNode<T>>,
    pub data: T,
}

pub struct BreadthFirstIterator<T> {
    queue: VecDeque<RcNode<T>>,
}

impl<T> BreadthFirstIterator<T> {
    pub fn new(root: RcNode<T>) -> Self {
        let mut queue = VecDeque::new();
        queue.push_back(root);
        BreadthFirstIterator { queue }
    }
}

impl<T> Iterator for BreadthFirstIterator<T> {
    type Item = (usize, RcNode<T>);
    fn next(&mut self) -> Option<Self::Item> {
        match self.queue.pop_front() {
            Some(ref n) => {
                for child in n.borrow().children() {
                    self.queue.push_back(child.clone());
                }
                let id = n.borrow().id;
                Some((id, n.clone()))
            }
            None => None,
        }
    }
}

pub struct BreadthFirstVisitor<T> {
    queue: VecDeque<RcNode<T>>,
}

impl<T> BreadthFirstVisitor<T> {
    pub fn new(root: RcNode<T>) -> Self {
        let mut queue = VecDeque::new();
        queue.push_back(root);
        BreadthFirstVisitor { queue }
    }
}

impl<T> NodeVisitor<T> for BreadthFirstVisitor<T> {
    fn visit<F: FnMut(&T) -> ()>(&mut self, func: F) {
        let mut func = func;
        if let Some(ref n) = self.queue.pop_front() {
            for child in n.borrow().children() {
                self.queue.push_back(child.clone());
            }
            (func)(&n.borrow().data);
        }
    }
    fn has_next(&self) -> bool {
        !self.queue.is_empty()
    }
}

impl<T> Drop for Node<T> {
    fn drop(&mut self) {
        println!("Dropping node {}", self.id);
    }
}

impl<T> fmt::Display for Node<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let p = match self.parent {
            Some(_) => "*",
            None => "*root",
        };
        write!(f, "{} ->(id: {})", p, self.id).expect("unable to display node");
        Ok(())
    }
}

impl<T> Node<T> {
    pub fn create(data: T, parent: Option<&RcNode<T>>) -> RcNode<T> {
        let prt = match parent {
            Some(ref p) => Some(Rc::downgrade(p)),
            None => None,
        };

        let node = Rc::new(RefCell::new(Node::new(data, prt)));

        if let Some(ref p) = parent {
            p.borrow_mut().add_child(node.clone());
        }
        node
    }

    pub fn find_root(node: RcNode<T>) -> RcNode<T> {
        match node.borrow().parent() {
            Some(p) => Node::find_root(p),
            None => node.clone(),
        }
    }

    fn new(data: T, parent: Option<WeakNode<T>>) -> Self {
        Node {
            id: GLOBAL_NODE_ID.fetch_add(1, Ordering::SeqCst),
            parent: match parent {
                Some(p) => Some(p),
                None => None,
            },
            children: Vec::new(),
            data,
        }
    }

    /// Recursively find if this node is a child of another
    pub fn is_child_of(this: RcNode<T>, parent: RcNode<T>) -> bool {
        match this.borrow().parent() {
            Some(p) => {
                if p.borrow().id == parent.borrow().id {
                    true
                } else {
                    Node::is_child_of(p, parent)
                }
            }
            None => false,
        }
    }

    pub fn reparent(child: RcNode<T>, target: RcNode<T>) -> Result<(), String> {
        if child.borrow().id == target.borrow().id {
            return Err("Cannot make node a child of itself.".to_string());
        }
        if !Node::is_child_of(target.clone(), child.clone()) {
            // check for cycles
            if let Some(old_parent) = child.borrow().parent() {
                old_parent.borrow_mut().remove_child(child.clone());
            }
            child.borrow_mut().parent = Some(Rc::downgrade(&target.clone()));
            target.borrow_mut().add_child(child);
            Ok(())
        } else {
            Err("Node cycle detected. Child is a parent of reparent target.".to_string())
            // format for better debug msg
        }
    }

    pub fn remove_child(&mut self, child: RcNode<T>) {
        let mut idx: Option<usize> = None;
        for i in 0usize..self.children.len() {
            let child_id = self.children[i].borrow().id;
            if child_id == child.borrow().id {
                idx = Some(i);
                break;
            }
        }
        if let Some(i) = idx {
            self.children.remove(i);
        }
    }

    pub fn find_child(&self, id: usize) -> Option<RcNode<T>> {
        let option: Option<&RcNode<T>> = self.children.iter().find(|x| x.borrow().id == id);
        match option {
            Some(obj_ref) => Some(obj_ref.clone()),
            None => None,
        }
    }

    pub fn siblings(&self) -> Option<Vec<RcNode<T>>> {
        match self.parent() {
            Some(p) => Some(
                p.borrow()
                    .children
                    .iter()
                    .filter(|x| x.borrow().id != self.id)
                    .cloned()
                    .collect(),
            ),
            None => None,
        }
    }

    pub fn children(&self) -> &Vec<RcNode<T>> {
        &self.children
    }

    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    // intentionally private for the time being (Node::new() specifies a parent)
    fn add_child(&mut self, child: RcNode<T>) {
        if self.id != child.borrow().id {
            self.children.push(child);
        }
    }

    pub fn parent(&self) -> Option<RcNode<T>> {
        match self.parent {
            Some(ref p) => Some(p.upgrade().unwrap()),
            None => None,
        }
    }

    pub fn debug_draw(&self, lvl: usize) {
        if lvl == 0 {
            println!("-- Hierarchy Dump --");
        }
        let c = if !self.children.is_empty() {
            "..."
        } else {
            ".leaf*"
        };
        println!(
            "{}{}{}",
            (0..lvl).map(|_| "....").collect::<String>(),
            self,
            c
        );
        for child in &self.children {
            child.borrow().debug_draw(lvl + 1);
        }
    }
}
