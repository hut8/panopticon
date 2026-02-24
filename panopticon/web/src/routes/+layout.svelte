<script lang="ts">
	import '../app.css';
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';

	let { children } = $props();

	interface AuthMe {
		id: string;
		email: string;
		email_confirmed: boolean;
		is_approved: boolean;
	}

	let user: AuthMe | null = $state(null);
	let authChecked = $state(false);
	let resendLoading = $state(false);
	let resendSent = $state(false);

	const publicPaths = ['/login', '/register', '/forgot-password', '/reset-password'];

	function isPublicPath(path: string): boolean {
		return publicPaths.some((p) => path === p || path.startsWith(p + '/'));
	}

	async function checkAuth() {
		try {
			const res = await fetch('/api/auth/me');
			if (res.ok) {
				user = await res.json();
			} else {
				user = null;
			}
		} catch {
			user = null;
		}
		authChecked = true;
	}

	async function resendConfirmation() {
		resendLoading = true;
		try {
			await fetch('/api/auth/resend-confirmation', { method: 'POST' });
			resendSent = true;
		} catch {
			// ignore
		} finally {
			resendLoading = false;
		}
	}

	async function handleLogout() {
		await fetch('/api/auth/logout', { method: 'POST' });
		user = null;
		goto('/login');
	}

	$effect(() => {
		checkAuth();
	});

	$effect(() => {
		if (!authChecked) return;
		const path = $page.url.pathname;
		if (!user && !isPublicPath(path)) {
			goto('/login');
		}
	});
</script>

<div class="flex min-h-screen flex-col bg-surface-950">
	{#if !authChecked}
		<main class="flex flex-1 items-center justify-center p-6">
			<p class="text-surface-400 animate-pulse">Loading...</p>
		</main>
	{:else if user && !user.email_confirmed}
		<main class="flex flex-1 items-center justify-center p-6">
			<div class="card preset-filled-surface-900 w-full max-w-sm space-y-5 p-8 text-center">
				<div class="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-warning-500/15">
					<svg class="h-7 w-7 text-warning-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
						<path stroke-linecap="round" stroke-linejoin="round" d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
					</svg>
				</div>
				<h1 class="h3">Check Your Email</h1>
				<p class="text-sm text-surface-400">
					We sent a confirmation link to <span class="font-medium text-surface-200">{user.email}</span>.
					Click the link to verify your account.
				</p>
				{#if resendSent}
					<p class="text-sm text-success-400">Confirmation email resent.</p>
				{:else}
					<button
						class="btn btn-base preset-outlined-surface-500 w-full"
						onclick={resendConfirmation}
						disabled={resendLoading}
					>
						{resendLoading ? 'Sending...' : 'Resend Email'}
					</button>
				{/if}
				<button class="text-sm text-surface-500 hover:text-surface-300 cursor-pointer" onclick={handleLogout}>
					Sign out
				</button>
			</div>
		</main>
	{:else if user && !user.is_approved}
		<main class="flex flex-1 items-center justify-center p-6">
			<div class="card preset-filled-surface-900 w-full max-w-sm space-y-5 p-8 text-center">
				<div class="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-tertiary-500/15">
					<svg class="h-7 w-7 text-tertiary-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
						<path stroke-linecap="round" stroke-linejoin="round" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
					</svg>
				</div>
				<h1 class="h3">Awaiting Approval</h1>
				<p class="text-sm text-surface-400">
					Your email is confirmed. An administrator needs to approve your account before you can continue.
				</p>
				<button class="text-sm text-surface-500 hover:text-surface-300 cursor-pointer" onclick={handleLogout}>
					Sign out
				</button>
			</div>
		</main>
	{:else}
		{@render children?.()}
	{/if}
</div>
