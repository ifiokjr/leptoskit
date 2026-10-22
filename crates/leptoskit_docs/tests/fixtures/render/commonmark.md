# The `title` of the page

The first paragraph has **strong**, _emphasis_, `inline code`, and a
hard break here\
followed by [a link](guide/next.md "Next page") and an ![inline image](assets/dot.svg).

## Lists

- a tight list
- with two items
  1. and a nested
  2. ordered list

3. an ordered list
4. that starts at three

- a loose list

- whose items are paragraphs

> A plain quote, with *markup*.

---

## Code

```rust title="src/main.rs"
fn main() {
	println!("<escaped>");
}
```

```toml
[app]
name = "demo"
```

```klingon
Qapla'
```

```
no language at all
```

## References

A [reference link][guide] and an ![image reference][dot].

[guide]: guide/next.md#setup "The guide"
[dot]: assets/dot.svg
