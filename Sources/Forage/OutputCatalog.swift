import Foundation

/// The fixed catalog of types Forage recipes are required to output.
///
/// A recipe doesn't get to invent its output shape — it picks fields from
/// these types and binds them via type-directed extraction. The runtime
/// collects emitted instances, validates required fields, and hands the
/// resulting `Snapshot` to the consumer (e.g. weed-prices) for persistence.
///
/// The catalog is currently shaped for cannabis-menu scraping (the first
/// downstream consumer). A future "shape this for a different domain" need
/// would mean either adding a parallel catalog or making the catalog
/// generic — explicit non-goal until a second consumer surfaces.
///
/// Relationships within a snapshot are referenced by `externalId`: a
/// `Product` declares its containing category via `categoryExternalId`,
/// each `Variant` lives inside its `Product.variants` array, each
/// `PriceObservation` points back at its variant via `variantExternalId`
/// scoped within the same product. The runtime resolves those references
/// to DB primary keys when persisting.

// MARK: - Dispensary

public struct ScrapedDispensary: Hashable, Codable, Sendable {
    public let slug: String
    public let name: String
    public let platform: String
    public let storeId: String?
    public let address: String?
    public let latitude: Double?
    public let longitude: Double?
    public let phone: String?
    public let website: String?
    public let timezone: String?

    public init(
        slug: String,
        name: String,
        platform: String,
        storeId: String? = nil,
        address: String? = nil,
        latitude: Double? = nil,
        longitude: Double? = nil,
        phone: String? = nil,
        website: String? = nil,
        timezone: String? = nil
    ) {
        self.slug = slug
        self.name = name
        self.platform = platform
        self.storeId = storeId
        self.address = address
        self.latitude = latitude
        self.longitude = longitude
        self.phone = phone
        self.website = website
        self.timezone = timezone
    }
}

// MARK: - Category

public struct ScrapedCategory: Hashable, Codable, Sendable {
    public let externalId: String
    public let name: String

    public init(externalId: String, name: String) {
        self.externalId = externalId
        self.name = name
    }
}

// MARK: - Product (with embedded Variants)

public struct ScrapedProduct: Hashable, Codable, Sendable {
    public let externalId: String
    public let name: String
    public let description: String?
    public let brand: String?
    public let strain: String?
    public let strainPrevalence: String?    // Indica / Sativa / Hybrid (normalized)
    public let productType: String?         // "Flower" / "Vape" / "Edible" etc.
    public let categoryExternalId: String?  // refs ScrapedCategory.externalId
    public let subcategoryExternalId: String?
    public let subcategoryName: String?
    public let terpenes: [String]
    public let images: [String]
    public let variants: [ScrapedVariant]

    public init(
        externalId: String,
        name: String,
        description: String? = nil,
        brand: String? = nil,
        strain: String? = nil,
        strainPrevalence: String? = nil,
        productType: String? = nil,
        categoryExternalId: String? = nil,
        subcategoryExternalId: String? = nil,
        subcategoryName: String? = nil,
        terpenes: [String] = [],
        images: [String] = [],
        variants: [ScrapedVariant] = []
    ) {
        self.externalId = externalId
        self.name = name
        self.description = description
        self.brand = brand
        self.strain = strain
        self.strainPrevalence = strainPrevalence
        self.productType = productType
        self.categoryExternalId = categoryExternalId
        self.subcategoryExternalId = subcategoryExternalId
        self.subcategoryName = subcategoryName
        self.terpenes = terpenes
        self.images = images
        self.variants = variants
    }
}

// MARK: - Variant

public struct ScrapedVariant: Hashable, Codable, Sendable {
    public let externalId: String
    public let name: String?
    public let sku: String?
    /// Size in canonical unit. Cannabis ounces are typically normalized to
    /// grams at scrape time (1oz → 28g) so per-gram math works regardless
    /// of how the dispensary labels its variants.
    public let sizeValue: Double?
    public let sizeUnit: String?            // "G" / "MG" / "ML" / "EA"

    public init(
        externalId: String,
        name: String? = nil,
        sku: String? = nil,
        sizeValue: Double? = nil,
        sizeUnit: String? = nil
    ) {
        self.externalId = externalId
        self.name = name
        self.sku = sku
        self.sizeValue = sizeValue
        self.sizeUnit = sizeUnit
    }
}

// MARK: - PriceObservation

public enum ScrapedMenuType: String, Codable, Hashable, Sendable {
    case recreational = "RECREATIONAL"
    case medical = "MEDICAL"
}

public struct ScrapedPriceObservation: Hashable, Codable, Sendable {
    /// Refs the parent product's `Variant.externalId` (scoped within
    /// `productExternalId` since variant external IDs are unique per
    /// product, not globally).
    public let productExternalId: String
    public let variantExternalId: String
    public let menuType: ScrapedMenuType
    public let price: Double?
    public let promoPrice: Double?
    public let availableQty: Double?
    public let thcPct: Double?
    public let cbdPct: Double?
    public let terpenePct: Double?
    public let stockType: String?
    public let availability: String?
    public let promoCount: Int?

    public init(
        productExternalId: String,
        variantExternalId: String,
        menuType: ScrapedMenuType,
        price: Double? = nil,
        promoPrice: Double? = nil,
        availableQty: Double? = nil,
        thcPct: Double? = nil,
        cbdPct: Double? = nil,
        terpenePct: Double? = nil,
        stockType: String? = nil,
        availability: String? = nil,
        promoCount: Int? = nil
    ) {
        self.productExternalId = productExternalId
        self.variantExternalId = variantExternalId
        self.menuType = menuType
        self.price = price
        self.promoPrice = promoPrice
        self.availableQty = availableQty
        self.thcPct = thcPct
        self.cbdPct = cbdPct
        self.terpenePct = terpenePct
        self.stockType = stockType
        self.availability = availability
        self.promoCount = promoCount
    }
}

// MARK: - Snapshot (the recipe's output unit)

/// The full output of one recipe run. The runtime hands this to the
/// downstream consumer for persistence; the consumer is responsible for
/// translating `externalId` references into its own primary keys and
/// upserting into its storage.
public struct Snapshot: Hashable, Codable, Sendable {
    public let dispensary: ScrapedDispensary
    public let categories: [ScrapedCategory]
    public let products: [ScrapedProduct]              // each contains [ScrapedVariant]
    public let observations: [ScrapedPriceObservation]
    public let observedAt: Date

    public init(
        dispensary: ScrapedDispensary,
        categories: [ScrapedCategory] = [],
        products: [ScrapedProduct] = [],
        observations: [ScrapedPriceObservation] = [],
        observedAt: Date = Date()
    ) {
        self.dispensary = dispensary
        self.categories = categories
        self.products = products
        self.observations = observations
        self.observedAt = observedAt
    }
}
