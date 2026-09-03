import "./globals.css"; import "@solana/wallet-adapter-react-ui/styles.css"; import {Providers} from "./providers";
export const metadata={title:"Civic Vote",description:"Permissionless token voting on Solana devnet"};
export default function Layout({children}:{children:React.ReactNode}){return <html lang="zh-CN"><body><Providers>{children}</Providers></body></html>}
