<script lang="ts">
	import '../app.css';

	let { children } = $props();
</script>

<div class="flex h-full min-h-screen flex-col">
	<main class="container mx-auto flex-1">
		{@render children?.()}
	</main>
</div>
